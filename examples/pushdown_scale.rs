//! One-off measurement: how much does pushdown save on a realistic collection?
//!
//! Not a test — timings are not a thing to assert on in CI. Run it by hand:
//!
//! ```sh
//! FIREUP_TEST_POSTGRES_URL=... cargo run --release --example pushdown_scale
//! ```
use std::time::Instant;

use fireup::local::postgres_store::PostgresStore;
use fireup::local::query::Query;
use fireup::local::store::{now_timestamp, Mutation, PendingWrite, Store};
use fireup::protos::google::firestore::v1::structured_query::{
    field_filter::Operator as FieldOperator, filter::FilterType, CollectionSelector, FieldFilter,
    FieldReference, Filter,
};
use fireup::protos::google::firestore::v1::value::ValueType;
use fireup::protos::google::firestore::v1::{Document, StructuredQuery, Value};

const DOCUMENTS: usize = 20_000;

#[tokio::main]
async fn main() {
    let url = std::env::var("FIREUP_TEST_POSTGRES_URL").expect("set FIREUP_TEST_POSTGRES_URL");
    let project = format!("scale-{}", &uuid::Uuid::new_v4().simple().to_string()[..8]);
    let root = format!("projects/{project}/databases/(default)/documents");
    let store = PostgresStore::connect(&url, 8).await.expect("connect");

    let text = |value: &str| Value {
        value_type: Some(ValueType::StringValue(value.to_string())),
    };

    let started = Instant::now();
    for chunk in (0..DOCUMENTS).collect::<Vec<_>>().chunks(500) {
        let writes = chunk
            .iter()
            .map(|index| {
                PendingWrite::new(Mutation::Put(Document {
                    name: format!("{root}/orders/order-{index:06}"),
                    fields: [
                        (
                            "status".to_string(),
                            text(if index % 1000 == 0 {
                                "refunded"
                            } else {
                                "shipped"
                            }),
                        ),
                        ("region".to_string(), text(&format!("r{}", index % 7))),
                        (
                            "total".to_string(),
                            Value {
                                value_type: Some(ValueType::IntegerValue((index % 5000) as i64)),
                            },
                        ),
                    ]
                    .into_iter()
                    .collect(),
                    create_time: None,
                    update_time: None,
                }))
            })
            .collect();
        store.commit(writes, now_timestamp()).await.expect("seed");
    }
    println!("seeded {DOCUMENTS} documents in {:?}", started.elapsed());

    let analyze = tokio_postgres::connect(&url, tokio_postgres::NoTls).await;
    if let Ok((client, connection)) = analyze {
        tokio::spawn(async move {
            let _ = connection.await;
        });
        let _ = client.execute("ANALYZE fireup_local_documents", &[]).await;
    }

    for (label, field, value) in [
        (
            "status == 'refunded' (1 in 1000)",
            "status",
            text("refunded"),
        ),
        ("region == 'r3' (1 in 7)", "region", text("r3")),
    ] {
        let structured = StructuredQuery {
            from: vec![CollectionSelector {
                collection_id: "orders".to_string(),
                all_descendants: false,
            }],
            r#where: Some(Filter {
                filter_type: Some(FilterType::FieldFilter(FieldFilter {
                    field: Some(FieldReference {
                        field_path: field.to_string(),
                    }),
                    op: FieldOperator::Equal as i32,
                    value: Some(value),
                })),
            }),
            ..Default::default()
        };
        let query = Query::compile(&structured).expect("compiles");

        // Warm both paths once so neither pays the first-connection cost.
        let _ = store.scan_collection(&root, "orders").await;
        let _ = store
            .scan_collection_filtered(&root, "orders", query.prefilter())
            .await;

        let at = Instant::now();
        let scanned = store.scan_collection(&root, "orders").await.expect("scan");
        let scan_time = at.elapsed();
        let scan_rows = scanned.len();
        let scan_results = query.evaluate(scanned).documents.len();

        let at = Instant::now();
        let pushed = store
            .scan_collection_filtered(&root, "orders", query.prefilter())
            .await
            .expect("pushed");
        let push_time = at.elapsed();
        let push_rows = pushed.len();
        let push_results = query.evaluate(pushed).documents.len();

        assert_eq!(scan_results, push_results, "{label} disagreed");
        println!(
            "{label}\n  full scan: {scan_rows:>6} rows read, {scan_time:>10.2?}\n  \
             pushed:    {push_rows:>6} rows read, {push_time:>10.2?}  ({:.1}x faster, {} results)",
            scan_time.as_secs_f64() / push_time.as_secs_f64().max(1e-9),
            push_results
        );
    }

    let (client, connection) = tokio_postgres::connect(&url, tokio_postgres::NoTls)
        .await
        .expect("cleanup connection");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    client
        .execute(
            "DELETE FROM fireup_local_documents WHERE project_id = $1",
            &[&project],
        )
        .await
        .expect("cleanup");
}
