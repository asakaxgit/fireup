#!/usr/bin/env python3
"""SDK conformance runner for `fireup local`, in Python.

The Node runner (`tests/conformance/node/runner.mjs`) is the required layer and
covers every row of the fidelity matrix. This one is the second-language check:
it drives the same server with Google's official *Python* libraries
(`google-cloud-firestore`, `google-cloud-datastore`) over the documented
`FIRESTORE_EMULATOR_HOST` / `DATASTORE_EMULATOR_HOST` variables.

It exists because the two libraries do not agree about which RPC implements
which call, how a value is serialized, or how a status maps to an exception. A
server that satisfies one SDK's idea of the contract can still fail the other,
and that is the kind of divergence our own Rust tests cannot find — they are
written from the same reading of the spec as the server.

It runs the subset of the matrix marked `"python": true`. The same rules as the
Node runner apply: the matrix drives the assertion, a status with no matching
probe branch is a failure rather than a skip, and coverage is enforced in both
directions over that subset.

    FIRESTORE_EMULATOR_HOST=127.0.0.1:4589 \
    DATASTORE_EMULATOR_HOST=127.0.0.1:4589 \
        python3 tests/conformance/python/conformance.py [--only SUBSTRING]

Exits 0 when every probed row matches the matrix, 1 otherwise, and 2 when the
official libraries are not installed (see requirements.txt) — a missing
optional dependency is reported as "not run", never as a pass.
"""

from __future__ import annotations

import argparse
import json
import os
import random
import sys
import time
import traceback
from pathlib import Path
from typing import Callable

MATRIX_PATH = Path(__file__).resolve().parents[1] / "matrix.json"
PROJECT_ID = os.environ.get("FIREUP_CONFORMANCE_PROJECT_ID", "fireup-local")

try:
    from google.api_core import exceptions as gexc
    from google.cloud import datastore, firestore
    from google.cloud.datastore.query import PropertyFilter
    from google.cloud.firestore_v1.base_query import FieldFilter
except ImportError as error:  # pragma: no cover - environment, not logic
    print(f"the official Python client libraries are not installed: {error}")
    print("install them with: pip install -r tests/conformance/python/requirements.txt")
    sys.exit(2)


# A run id in every resource name, so a second run against a persistent
# Postgres store starts clean instead of tripping over the last run's data.
RUN_ID = f"{int(time.time()):x}{random.randrange(10_000):04d}"

PROBES: dict[str, dict[str, Callable]] = {}


def probe(feature_id: str, status: str):
    """Register the branch that proves `feature_id` really is `status`."""

    def register(fn):
        PROBES.setdefault(feature_id, {})[status] = fn
        return fn

    return register


class Context:
    def __init__(self, db: firestore.Client, ds: datastore.Client):
        self.db = db
        self.ds = ds
        self.project_id = PROJECT_ID
        self.run_id = RUN_ID

    def fresh_collection(self, label: str):
        safe = "".join(c if c.isalnum() else "_" for c in label)
        return self.db.collection(f"confpy_{RUN_ID}_{safe}")

    def fresh_kind(self, label: str) -> str:
        safe = "".join(c if c.isalnum() else "_" for c in label)
        return f"ConfPy_{RUN_ID}_{safe}"


def expect_unimplemented(fn: Callable, names: str | None = None):
    """Assert a call is refused with UNIMPLEMENTED and a message naming the gap."""
    try:
        result = fn()
        # Several Python query calls are lazy: consuming the iterator is what
        # sends the RPC.
        if hasattr(result, "__iter__") and not isinstance(result, (list, tuple, dict, str)):
            list(result)
    # google-api-core names gRPC UNIMPLEMENTED after its HTTP mapping: 501.
    except gexc.MethodNotImplemented as error:
        message = str(error)
        assert "fireup local does not serve" in message or "fireup local does not support" in message, (
            f"the refusal must name fireup and the gap, got: {message}"
        )
        if names is not None:
            assert names.lower() in message.lower(), (
                f"the refusal must name {names!r}, got: {message}"
            )
        return error
    except Exception as error:  # noqa: BLE001 - we are classifying the failure
        raise AssertionError(
            f"expected UNIMPLEMENTED, got {type(error).__name__}: {error}"
        ) from error
    raise AssertionError("expected the call to be refused, but it succeeded")


def ids(snapshots) -> list[str]:
    return [snapshot.id for snapshot in snapshots]


def seed_cities(collection):
    collection.document("tokyo").set({"country": "JP", "population": 37, "tags": ["capital", "big"]})
    collection.document("osaka").set({"country": "JP", "population": 19, "tags": ["big"]})
    collection.document("kyoto").set({"country": "JP", "population": 15, "tags": ["old"]})
    collection.document("seoul").set({"country": "KR", "population": 25, "tags": ["capital"]})
    return collection


def seed_tasks(ds: datastore.Client, kind: str) -> str:
    entities = []
    for name, priority, owner in (("a", 1, "ada"), ("b", 2, "ada"), ("c", 3, "bob"), ("d", 4, "bob")):
        entity = datastore.Entity(key=ds.key(kind, name))
        entity.update({"priority": priority, "owner": owner})
        entities.append(entity)
    ds.put_multi(entities)
    return kind


# ---- Firestore RPCs --------------------------------------------------------


@probe("firestore.ListDocuments", "served")
def _list_documents(ctx: Context):
    cities = ctx.fresh_collection("listdocuments")
    for name in ("a", "b", "c"):
        cities.document(name).set({"n": 1})
    cities.document("ghost").collection("kids").document("k").set({"n": 0})

    found = sorted(ref.id for ref in cities.list_documents())
    assert found == ["a", "b", "c", "ghost"], f"got {found}"


@probe("firestore.BatchGetDocuments", "served")
def _batch_get(ctx: Context):
    users = ctx.fresh_collection("batchget")
    users.document("found").set({"name": "Ada"})

    snapshots = {s.id: s for s in ctx.db.get_all([users.document("found"), users.document("nobody")])}
    assert snapshots["found"].exists is True
    assert snapshots["found"].get("name") == "Ada"
    assert snapshots["nobody"].exists is False, "a missing document is reported, not an error"


@probe("firestore.BeginTransaction", "served")
def _begin_transaction(ctx: Context):
    ref = ctx.fresh_collection("begintransaction").document("c")
    ref.set({"n": 1})

    @firestore.transactional
    def bump(transaction):
        snapshot = ref.get(transaction=transaction)
        nxt = snapshot.get("n") + 1
        transaction.update(ref, {"n": nxt})
        return nxt

    assert bump(ctx.db.transaction()) == 2
    assert ref.get().get("n") == 2


@probe("firestore.Commit", "served")
def _commit(ctx: Context):
    cities = ctx.fresh_collection("commit")
    cities.document("doomed").set({"n": 0})

    batch = ctx.db.batch()
    batch.set(cities.document("created"), {"n": 1})
    batch.update(cities.document("doomed"), {"n": 2})
    batch.delete(cities.document("doomed"))
    results = batch.commit()

    assert len(results) == 3, f"one WriteResult per write, got {len(results)}"
    assert cities.document("created").get().get("n") == 1
    assert cities.document("doomed").get().exists is False


@probe("firestore.Rollback", "served")
def _rollback(ctx: Context):
    ref = ctx.fresh_collection("rollback").document("kyoto")
    ref.set({"n": 15})

    class Abandon(Exception):
        pass

    @firestore.transactional
    def abandon(transaction):
        snapshot = ref.get(transaction=transaction)
        transaction.update(ref, {"n": snapshot.get("n") + 100})
        raise Abandon

    try:
        abandon(ctx.db.transaction())
    except Abandon:
        pass
    else:
        raise AssertionError("the transaction body should have raised")

    assert ref.get().get("n") == 15, "the rolled-back write must not land"


@probe("firestore.RunQuery", "served")
def _run_query(ctx: Context):
    cities = seed_cities(ctx.fresh_collection("runquery"))

    snapshots = list(
        cities.where(filter=FieldFilter("country", "==", "JP"))
        .order_by("population", direction=firestore.Query.DESCENDING)
        .stream()
    )
    assert ids(snapshots) == ["tokyo", "osaka", "kyoto"], f"got {ids(snapshots)}"
    assert snapshots[0].create_time is not None, "results carry their document metadata"


@probe("firestore.ListCollectionIds", "served")
def _list_collection_ids(ctx: Context):
    cities = ctx.fresh_collection("listcollectionids")
    cities.document("tokyo").collection("districts").document("d1").set({"n": 1})

    root = [collection.id for collection in ctx.db.collections()]
    assert cities.id in root, f"expected {cities.id} among {len(root)} root collections"

    nested = [c.id for c in cities.document("tokyo").collections()]
    assert nested == ["districts"], f"got {nested}"


@probe("firestore.RunAggregationQuery", "unimplemented")
def _aggregation_rpc(ctx: Context):
    cities = seed_cities(ctx.fresh_collection("aggregationrpc"))
    expect_unimplemented(lambda: cities.count().get(), names="Aggregation")


@probe("firestore.RunAggregationQuery", "served")
def _aggregation_rpc_served(ctx: Context):
    cities = seed_cities(ctx.fresh_collection("aggregationrpc"))
    result = cities.count().get()
    assert int(result[0][0].value) == 4


# ---- queries ---------------------------------------------------------------


@probe("query.comparison-operators", "served")
def _comparisons(ctx: Context):
    cities = seed_cities(ctx.fresh_collection("cmp"))

    def run(field, op, value, order=None):
        query = cities.where(filter=FieldFilter(field, op, value))
        if order:
            query = query.order_by(order)
        return sorted(ids(query.stream()))

    assert run("country", "==", "JP") == ["kyoto", "osaka", "tokyo"]
    assert run("country", "!=", "JP") == ["seoul"]
    assert run("population", "<", 19) == ["kyoto"]
    assert run("population", "<=", 19) == ["kyoto", "osaka"]
    assert run("population", ">", 25) == ["tokyo"]
    assert run("population", ">=", 25) == ["seoul", "tokyo"]


@probe("query.membership-and-array-operators", "served")
def _membership(ctx: Context):
    cities = seed_cities(ctx.fresh_collection("membership"))

    def run(field, op, value):
        return sorted(ids(cities.where(filter=FieldFilter(field, op, value)).stream()))

    assert run("country", "in", ["KR", "TW"]) == ["seoul"]
    assert run("country", "not-in", ["JP"]) == ["seoul"]
    assert run("tags", "array_contains", "capital") == ["seoul", "tokyo"]
    assert run("tags", "array_contains_any", ["old", "capital"]) == ["kyoto", "seoul", "tokyo"]


@probe("query.order-by", "served")
def _order_by(ctx: Context):
    cities = seed_cities(ctx.fresh_collection("orderby"))

    assert ids(cities.order_by("population").stream()) == ["kyoto", "osaka", "seoul", "tokyo"]
    assert ids(cities.order_by("population", direction=firestore.Query.DESCENDING).stream()) == [
        "tokyo",
        "seoul",
        "osaka",
        "kyoto",
    ]
    several = cities.order_by("country").order_by(
        "population", direction=firestore.Query.DESCENDING
    )
    assert ids(several.stream()) == ["tokyo", "osaka", "kyoto", "seoul"]


@probe("query.limit-offset", "served")
def _limit_offset(ctx: Context):
    cities = seed_cities(ctx.fresh_collection("limitoffset"))

    assert ids(cities.order_by("population").limit(2).stream()) == ["kyoto", "osaka"]
    assert ids(cities.order_by("population").offset(2).stream()) == ["seoul", "tokyo"]
    assert ids(cities.order_by("population").offset(1).limit(2).stream()) == ["osaka", "seoul"]


@probe("query.collection-group", "unimplemented")
def _collection_group(ctx: Context):
    cities = ctx.fresh_collection("collectiongroup")
    group = f"kidspy_{RUN_ID}"
    cities.document("tokyo").collection(group).document("k").set({"n": 1})

    expect_unimplemented(lambda: ctx.db.collection_group(group).stream(), names="collection group")


@probe("query.collection-group", "served")
def _collection_group_served(ctx: Context):
    cities = ctx.fresh_collection("collectiongroup")
    group = f"kidspy_{RUN_ID}"
    cities.document("tokyo").collection(group).document("k").set({"n": 1})

    assert ids(ctx.db.collection_group(group).stream()) == ["k"]


@probe("query.cursors", "unimplemented")
def _cursors(ctx: Context):
    cities = seed_cities(ctx.fresh_collection("cursors"))

    expect_unimplemented(
        lambda: cities.order_by("population").start_at({"population": 19}).stream(),
        names="cursor",
    )
    expect_unimplemented(
        lambda: cities.order_by("population").end_before({"population": 25}).stream(),
        names="cursor",
    )


@probe("query.cursors", "served")
def _cursors_served(ctx: Context):
    cities = seed_cities(ctx.fresh_collection("cursors"))

    started = cities.order_by("population").start_at({"population": 19}).stream()
    assert ids(started) == ["osaka", "seoul", "tokyo"]


@probe("query.nested-field-paths", "unimplemented")
def _nested_paths(ctx: Context):
    docs = ctx.fresh_collection("nestedpaths")
    docs.document("d1").set({"a": {"b": 1}, "keep": True})

    expect_unimplemented(
        lambda: docs.where(filter=FieldFilter("a.b", "==", 1)).stream(),
        names="nested field path",
    )
    expect_unimplemented(lambda: docs.order_by("a.b").stream(), names="nested field path")
    expect_unimplemented(
        lambda: docs.document("d1").update({"a.b": 2}), names="nested field path"
    )

    assert docs.document("d1").get().to_dict() == {"a": {"b": 1}, "keep": True}, (
        "a refused nested-path write must leave the document untouched"
    )


@probe("query.aggregations", "unimplemented")
def _aggregations(ctx: Context):
    cities = seed_cities(ctx.fresh_collection("aggregate"))

    expect_unimplemented(lambda: cities.sum("population").get(), names="Aggregation")
    expect_unimplemented(lambda: cities.avg("population").get(), names="Aggregation")


@probe("query.aggregations", "served")
def _aggregations_served(ctx: Context):
    cities = seed_cities(ctx.fresh_collection("aggregate"))
    assert int(cities.sum("population").get()[0][0].value) == 96


@probe("query.nested-field-paths", "served")
def _nested_paths_served(ctx: Context):
    docs = ctx.fresh_collection("nestedpaths")
    docs.document("kyoto").set({"address": {"city": "Kyoto", "zip": "600"}, "rank": 1})
    docs.document("osaka").set({"address": {"city": "Osaka", "zip": "530"}, "rank": 2})

    found = ids(docs.where(filter=FieldFilter("address.city", "==", "Osaka")).stream())
    assert found == ["osaka"], f"got {found}"

    ordered = ids(docs.order_by("address.city", direction=firestore.Query.DESCENDING).stream())
    assert ordered == ["osaka", "kyoto"], f"got {ordered}"

    projected = list(docs.select(["address.city"]).order_by("__name__").stream())
    assert projected[0].to_dict() == {"address": {"city": "Kyoto"}}, projected[0].to_dict()


# ---- field transforms ------------------------------------------------------


@probe("transform.server-timestamp", "served")
def _server_timestamp(ctx: Context):
    ref = ctx.fresh_collection("servertimestamp").document("d")
    ref.set({"n": 1})
    ref.update({"at": firestore.SERVER_TIMESTAMP, "also": firestore.SERVER_TIMESTAMP})

    snapshot = ref.get()
    at = snapshot.get("at")
    assert at is not None, "SERVER_TIMESTAMP must have written a timestamp"
    assert at == snapshot.get("also"), "every field transformed in one commit gets the same value"
    assert at.microsecond % 1000 == 0, f"REQUEST_TIME has millisecond precision, got {at!r}"


@probe("transform.increment", "served")
def _increment(ctx: Context):
    ref = ctx.fresh_collection("increment").document("d")
    ref.set({"n": 10})

    ref.update({"n": firestore.Increment(5)})
    assert ref.get().get("n") == 15

    # A double on either side makes the sum a double.
    ref.update({"n": firestore.Increment(0.5)})
    assert ref.get().get("n") == 15.5


@probe("transform.array-union", "served")
def _array_union(ctx: Context):
    ref = ctx.fresh_collection("arrayunion").document("d")
    ref.set({"tags": ["a"]})

    # "a" is already present and "b" is repeated: exactly one "b" is appended.
    ref.update({"tags": firestore.ArrayUnion(["a", "b", "b"])})
    assert ref.get().get("tags") == ["a", "b"], ref.get().get("tags")


@probe("transform.array-remove", "served")
def _array_remove(ctx: Context):
    ref = ctx.fresh_collection("arrayremove").document("d")
    ref.set({"tags": ["a", "b", "a"]})

    ref.update({"tags": firestore.ArrayRemove(["a"])})
    assert ref.get().get("tags") == ["b"], "every occurrence is removed"


# ---- transactions ----------------------------------------------------------


@probe("transaction.optimistic-concurrency", "served")
def _optimistic_concurrency(ctx: Context):
    ref = ctx.fresh_collection("occ").document("osaka")
    ref.set({"n": 19})
    attempts = {"count": 0}

    @firestore.transactional
    def bump(transaction):
        attempts["count"] += 1
        snapshot = ref.get(transaction=transaction)
        if attempts["count"] == 1:
            # A write from outside invalidates the read set, so the commit must
            # be ABORTED — which the library turns into a retry. Without the
            # abort, this write would be silently lost.
            ref.update({"n": 20})
        transaction.update(ref, {"n": snapshot.get("n") + 1})

    bump(ctx.db.transaction())

    assert attempts["count"] > 1, f"the commit should have aborted and retried, attempts={attempts}"
    assert ref.get().get("n") == 21, "the retry must read the concurrent write"


# ---- Datastore -------------------------------------------------------------


@probe("datastore.Lookup", "served")
def _ds_lookup(ctx: Context):
    kind = ctx.fresh_kind("lookup")
    key = ctx.ds.key(kind, "one")
    entity = datastore.Entity(key=key)
    entity.update({"title": "look me up", "priority": 1})
    ctx.ds.put(entity)

    fetched = ctx.ds.get(key)
    assert fetched is not None, "Lookup must find a saved entity"
    assert fetched["title"] == "look me up"
    assert fetched.key.name == "one"
    assert ctx.ds.get(ctx.ds.key(kind, "nobody")) is None, "a missing entity is not an error"


@probe("datastore.RunQuery", "served")
def _ds_run_query(ctx: Context):
    kind = seed_tasks(ctx.ds, ctx.fresh_kind("runquery"))

    def run(query, **kwargs):
        return [entity.key.name for entity in query.fetch(**kwargs)]

    equal = ctx.ds.query(kind=kind)
    equal.add_filter(filter=PropertyFilter("priority", "=", 2))
    assert run(equal) == ["b"]

    less = ctx.ds.query(kind=kind, order=["priority"])
    less.add_filter(filter=PropertyFilter("priority", "<", 3))
    assert run(less) == ["a", "b"]

    greater = ctx.ds.query(kind=kind, order=["priority"])
    greater.add_filter(filter=PropertyFilter("priority", ">=", 3))
    assert run(greater) == ["c", "d"]

    conjunction = ctx.ds.query(kind=kind)
    conjunction.add_filter(filter=PropertyFilter("owner", "=", "bob"))
    conjunction.add_filter(filter=PropertyFilter("priority", "=", 4))
    assert run(conjunction) == ["d"]

    assert run(ctx.ds.query(kind=kind, order=["priority"])) == ["a", "b", "c", "d"]
    assert run(ctx.ds.query(kind=kind, order=["-priority"])) == ["d", "c", "b", "a"]
    assert run(ctx.ds.query(kind=kind, order=["priority"]), limit=2) == ["a", "b"]
    assert run(ctx.ds.query(kind=kind, order=["priority"]), offset=2) == ["c", "d"]

    # HAS_ANCESTOR, through an ancestor key path.
    parent_kind = f"{kind}Parent"
    parent = ctx.ds.key(parent_kind, "p1")
    parent_entity = datastore.Entity(key=parent)
    parent_entity.update({"label": "parent"})
    child = datastore.Entity(key=ctx.ds.key(parent_kind, "p1", kind, "child"))
    child.update({"priority": 9})
    ctx.ds.put_multi([parent_entity, child])

    descendants = ctx.ds.query(kind=kind, ancestor=parent)
    assert run(descendants) == ["child"], "HAS_ANCESTOR scopes the kind to one parent"


@probe("datastore.Commit", "served")
def _ds_commit(ctx: Context):
    kind = ctx.fresh_kind("commit")

    # The Python library's put() is an upsert; it has no separate insert/update,
    # so that distinction is asserted by the Node runner instead.
    key = ctx.ds.key(kind, "one")
    entity = datastore.Entity(key=key)
    entity.update({"n": 1})
    ctx.ds.put(entity)
    entity.update({"n": 2})
    ctx.ds.put(entity)
    assert ctx.ds.get(key)["n"] == 2

    ctx.ds.delete(key)
    assert ctx.ds.get(key) is None

    # An incomplete key is completed by the commit with a numeric id.
    incomplete = datastore.Entity(key=ctx.ds.key(kind))
    incomplete.update({"n": 7})
    ctx.ds.put(incomplete)
    assert incomplete.key.id, "the commit must report the allocated id back"
    assert ctx.ds.get(ctx.ds.key(kind, incomplete.key.id))["n"] == 7


@probe("datastore.BeginTransaction-Rollback", "served")
def _ds_transaction(ctx: Context):
    kind = ctx.fresh_kind("tx")
    key = ctx.ds.key(kind, "one")
    entity = datastore.Entity(key=key)
    entity.update({"n": 1})
    ctx.ds.put(entity)

    with ctx.ds.transaction():
        current = ctx.ds.get(key)
        current["n"] = current["n"] + 1
        ctx.ds.put(current)
    assert ctx.ds.get(key)["n"] == 2

    class Abandon(Exception):
        pass

    try:
        with ctx.ds.transaction():
            doomed = ctx.ds.get(key)
            doomed["n"] = 99
            ctx.ds.put(doomed)
            raise Abandon
    except Abandon:
        pass
    assert ctx.ds.get(key)["n"] == 2, "a rolled-back write must not land"


@probe("datastore.AllocateIds", "served")
def _ds_allocate_ids(ctx: Context):
    kind = ctx.fresh_kind("allocate")
    keys = ctx.ds.allocate_ids(ctx.ds.key(kind), 3)

    assert len(keys) == 3
    assert all(key.id for key in keys), f"every key must be complete, got {keys}"
    assert len({key.id for key in keys}) == 3, "allocated ids must be distinct"

    entity = datastore.Entity(key=keys[0])
    entity.update({"n": 1})
    ctx.ds.put(entity)
    assert ctx.ds.get(keys[0])["n"] == 1


# ---- runner ----------------------------------------------------------------


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--only", help="run only features whose id contains this substring")
    args = parser.parse_args()

    firestore_host = os.environ.get("FIRESTORE_EMULATOR_HOST")
    datastore_host = os.environ.get("DATASTORE_EMULATOR_HOST")
    if not firestore_host or not datastore_host:
        print("FIRESTORE_EMULATOR_HOST and DATASTORE_EMULATOR_HOST must both be set")
        return 2

    matrix = json.loads(MATRIX_PATH.read_text())
    features = [feature for feature in matrix["features"] if feature.get("python")]

    orphans = sorted(set(PROBES) - {feature["id"] for feature in features})

    print(f"Driving fireup local at {firestore_host} / {datastore_host} with the Python SDKs")
    print(f"Matrix: {len(features)} feature(s) marked python:true")
    print(f"Run id: {RUN_ID}\n")

    context = Context(
        firestore.Client(project=PROJECT_ID),
        datastore.Client(project=PROJECT_ID),
    )

    failures: list[tuple[dict, str]] = []
    ran = 0
    for feature in features:
        if args.only and args.only not in feature["id"]:
            continue
        ran += 1
        status = feature["status"]
        branches = PROBES.get(feature["id"])
        if branches is None:
            print(f"  MISSING   {feature['id']}")
            failures.append(
                (
                    feature,
                    f"no Python probe for {feature['id']}. Either write one or set "
                    f'"python": false on that matrix row.',
                )
            )
            continue
        branch = branches.get(status)
        if branch is None:
            print(f"  NO-BRANCH {feature['id']} ({status})")
            failures.append(
                (
                    feature,
                    f"the matrix says {feature['id']} is {status}, but its Python probe has "
                    f"no {status} branch. Write the check that proves the new status.",
                )
            )
            continue

        try:
            branch(context)
            print(f"  ok        {feature['id']} [{status}]")
        except Exception:  # noqa: BLE001 - a probe failure is the result
            print(f"  FAIL      {feature['id']} [{status}]")
            failures.append((feature, traceback.format_exc()))

    print(f"\n{ran - len(failures)}/{ran} matrix features verified with the Python SDKs")
    if orphans:
        print(f"\nPython probes with no python:true matrix row: {', '.join(orphans)}")

    if failures or orphans:
        print("\nFailures:")
        for feature, detail in failures:
            print(f"- {feature['id']} [{feature['status']}] — {feature['label']}")
            print(f"  {detail}")
        return 1

    print("\nfireup local speaks the published contract to the Python SDKs too.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
