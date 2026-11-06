#!/usr/bin/env python3
"""Generate managed-export fixtures for the *edge cases* the base fixtures miss.

`generate_datastore_export.py` produces a representative happy-path export: one
`output-0` per kind, the default namespace, and a well-behaved dataset. Real
managed exports are messier, and every gap below has silently cost a migration
somewhere:

  * a kind is sharded across `output-0`, `output-1`, … `output-10`, so a reader
    that stops at the first shard loses most of the data and still exits 0
  * a record larger than two 32 KiB blocks produces a `MIDDLE` fragment, a
    record type the base fixtures never emit
  * an entity key is `(kind, int64 id)` *or* `(kind, string name)`, and the two
    are distinct identities even when they render to the same text
  * a non-default namespace lives under `namespace_<ns>/kind_<Kind>/`, and the
    same key name may exist in several namespaces at once
  * `int64` and `double` properties reach their limits, including non-finite
    doubles and pre-epoch timestamps
  * property names collide once PostgreSQL truncates identifiers to 63 bytes

This writes two trees with the same logical dataset, mirroring the base
fixtures' split so a framing bug can be told apart from a payload bug:

  export-edge-onestore/   legacy `storage_onestore_v3.EntityProto` payloads
  export-edge-v1/         `google.datastore.v1.Entity` payloads

Usage:
    python3 tests/fixtures/generate_edge_export.py [output-dir]
"""

from __future__ import annotations

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from generate_datastore_export import (  # noqa: E402
    BLOCK_SIZE,
    HEADER_SIZE,
    MEANING_BLOB,
    MEANING_EMPTY_LIST,
    MEANING_ENTITY_PROTO,
    MEANING_GD_WHEN,
    MEANING_NO_MEANING,
    MEANING_TEXT,
    APP_ID,
    f_bytes,
    f_string,
    f_varint,
    frame_records,
    onestore_path,
    onestore_property,
    onestore_reference,
    pv_bool,
    pv_double,
    pv_int,
    pv_string,
    v1_array,
    v1_bool,
    v1_double,
    v1_entity_value,
    v1_int,
    v1_string,
    v1_timestamp,
    PV_NULL,
)

I64_MAX = (1 << 63) - 1
I64_MIN = -(1 << 63)

# 2026-03-14T09:26:53.589793Z, matching the base fixtures.
TS_SECONDS = 1773480413
# 1969-07-20T20:17:40Z — before the Unix epoch, so micros/seconds go negative.
PRE_EPOCH_SECONDS = -14182940
# 2286-11-20T17:46:40Z — past the 2^31 second mark that breaks 32-bit readers.
FAR_FUTURE_SECONDS = 10_000_000_000

# Spans three blocks, so framing must emit FIRST + MIDDLE + LAST.
HUGE_BLOB = bytes((i * 17 + 3) & 0xFF for i in range(80_000))

# A 70-character name is longer than PostgreSQL's 63-byte identifier limit, and
# these two share their first 63 characters. Truncating without disambiguating
# collapses them into one column and drops a field.
LONG_PREFIX = "shipping_address_normalization_confidence_score_recomputed_at_r"
assert len(LONG_PREFIX) == 63
LONG_NAME_A = LONG_PREFIX + "evisio"
LONG_NAME_B = LONG_PREFIX + "equest"
assert len(LONG_NAME_A) == len(LONG_NAME_B) == 69
assert LONG_NAME_A[:63] == LONG_NAME_B[:63] and LONG_NAME_A != LONG_NAME_B


# --------------------------------------------------------------------------
# namespace-aware helpers (the base generator only writes the default namespace)
# --------------------------------------------------------------------------


def onestore_entity_ns(
    elements: list[tuple[str, object]],
    properties: list[bytes],
    namespace: str = "",
    raw_properties: list[bytes] | None = None,
) -> bytes:
    """EntityProto { key = 13; property = 14; raw_property = 15; entity_group = 16; }"""
    out = bytearray(f_bytes(13, onestore_reference(elements, namespace)))
    out += f_bytes(16, onestore_path(elements[:1]))
    for prop in sorted(properties):
        out += f_bytes(14, prop)
    for prop in raw_properties or []:
        out += f_bytes(15, prop)
    return bytes(out)


def v1_key_ns(
    elements: list[tuple[str, object]],
    namespace: str = "",
    project: str = "fireup-fixture",
) -> bytes:
    """Key { partition_id = 1; repeated PathElement path = 2; }

    `PartitionId { project_id = 2; namespace_id = 4; }`
    """
    partition = bytearray(f_string(2, project))
    if namespace:
        partition += f_string(4, namespace)
    out = bytearray(f_bytes(1, bytes(partition)))
    for kind, ident in elements:
        element = bytearray(f_string(1, kind))
        if isinstance(ident, int):
            element += f_varint(2, ident)
        else:
            element += f_string(3, str(ident))
        out += f_bytes(2, bytes(element))
    return bytes(out)


def v1_entity_ns(
    elements: list[tuple[str, object]],
    properties: dict[str, bytes],
    namespace: str = "",
) -> bytes:
    out = bytearray(f_bytes(1, v1_key_ns(elements, namespace)))
    for name in sorted(properties):
        out += f_bytes(3, f_string(1, name) + f_bytes(2, properties[name]))
    return bytes(out)


def v1_blob_value(value: bytes) -> bytes:
    return f_bytes(18, value)


def v1_key_value_ns(elements: list[tuple[str, object]], namespace: str = "") -> bytes:
    return f_bytes(5, v1_key_ns(elements, namespace))


# --------------------------------------------------------------------------
# Shards — one kind spread over output-0, output-1, output-2, output-10
# --------------------------------------------------------------------------

# Deliberately shaped so a reader that only reads output-0, or that sorts shard
# files as text, produces a wrong answer: output-10 sorts before output-2
# lexically, and `sequence` pins the expected read order.
SHARDS = [
    {"id": "s0a", "shard": 0, "sequence": 1},
    {"id": "s0b", "shard": 0, "sequence": 2},
    {"id": "s1a", "shard": 1, "sequence": 3},
    {"id": "s1b", "shard": 1, "sequence": 4},
    {"id": "s2a", "shard": 2, "sequence": 5},
    {"id": "s10a", "shard": 10, "sequence": 6},
]


def onestore_shard(row: dict) -> bytes:
    return onestore_entity_ns(
        [("Shards", row["id"])],
        [
            onestore_property("sequence", pv_int(row["sequence"])),
            onestore_property("shard", pv_int(row["shard"])),
        ],
    )


def v1_shard(row: dict) -> bytes:
    return v1_entity_ns(
        [("Shards", row["id"])],
        {"sequence": v1_int(row["sequence"]), "shard": v1_int(row["shard"])},
    )


# --------------------------------------------------------------------------
# Fragments — block-boundary framing
# --------------------------------------------------------------------------


def _sized_entity(build, target: int) -> bytes:
    """Serialise `build(filler_bytes)` to exactly `target` bytes.

    Growing the filler grows the record by the same amount except when the
    length varint widens, so solve by measurement rather than arithmetic.
    """
    filler_len = 0
    for _ in range(8):
        candidate = build(b"\x5a" * filler_len)
        delta = target - len(candidate)
        if delta == 0:
            return candidate
        if filler_len + delta < 0:
            raise ValueError(f"cannot shrink below {len(candidate)} bytes to reach {target}")
        filler_len += delta
    raise ValueError(f"did not converge on {target} bytes")


def onestore_fragments() -> list[bytes]:
    huge = onestore_entity_ns(
        [("Fragments", "three-block-blob")],
        [
            onestore_property("payload", pv_string(HUGE_BLOB), MEANING_BLOB),
            onestore_property("size", pv_int(len(HUGE_BLOB))),
        ],
    )
    # A record whose body exactly fills a fresh block, so the next record starts
    # on a block boundary with zero slack and no padding.
    exact = _sized_entity(
        lambda filler: onestore_entity_ns(
            [("Fragments", "exact-block-fill")],
            [
                onestore_property("filler", pv_string(filler), MEANING_BLOB),
                onestore_property("kind_of_test", pv_string("exact"), MEANING_TEXT),
            ],
        ),
        BLOCK_SIZE - HEADER_SIZE,
    )
    small = onestore_entity_ns(
        [("Fragments", "after-the-boundary")],
        [onestore_property("kind_of_test", pv_string("small"), MEANING_TEXT)],
    )
    # `exact` goes first so it actually starts on a block boundary; `huge` goes
    # last so its FIRST fragment begins mid-block, which is where a reader that
    # mishandles the remaining-room calculation goes wrong.
    return [exact, small, huge]


def v1_fragments() -> list[bytes]:
    huge = v1_entity_ns(
        [("Fragments", "three-block-blob")],
        {"payload": v1_blob_value(HUGE_BLOB), "size": v1_int(len(HUGE_BLOB))},
    )
    exact = _sized_entity(
        lambda filler: v1_entity_ns(
            [("Fragments", "exact-block-fill")],
            {"filler": v1_blob_value(filler), "kind_of_test": v1_string("exact")},
        ),
        BLOCK_SIZE - HEADER_SIZE,
    )
    small = v1_entity_ns(
        [("Fragments", "after-the-boundary")],
        {"kind_of_test": v1_string("small")},
    )
    return [exact, small, huge]


# --------------------------------------------------------------------------
# Idents — key identity edge cases
# --------------------------------------------------------------------------

# `label` is the only property, and it is unique per entity, so a test can tell
# which entities survived a round trip even when their keys render alike.
IDENTS = [
    # Distinct identities that render to the same `Idents/1001` text. Datastore
    # keeps them apart (one is an int64 id, one is a string name); anything that
    # keys rows on the rendered path alone silently merges them.
    {"path": [("Idents", 1001)], "label": "numeric-id-1001"},
    {"path": [("Idents", "1001")], "label": "string-name-1001"},
    # A string name containing slashes renders like a two-element path.
    {"path": [("Idents", "a/Idents/9")], "label": "slash-in-name"},
    # Negative and maximal int64 ids are both legal.
    {"path": [("Idents", -5)], "label": "negative-id"},
    {"path": [("Idents", I64_MAX)], "label": "max-int64-id"},
    # Three ancestor levels; the base fixtures stop at two.
    {"path": [("Org", "acme"), ("Team", 42), ("Idents", "deep")], "label": "three-level-ancestor"},
]


def onestore_ident(row: dict) -> bytes:
    return onestore_entity_ns(
        row["path"],
        [onestore_property("label", pv_string(row["label"]), MEANING_TEXT)],
    )


def v1_ident(row: dict) -> bytes:
    return v1_entity_ns(row["path"], {"label": v1_string(row["label"])})


# --------------------------------------------------------------------------
# Values — value-type edge cases
# --------------------------------------------------------------------------

VALUES = [
    {
        "id": "limits",
        "big": I64_MAX,
        "small": I64_MIN,
        "ratio": -0.0,
        "when": PRE_EPOCH_SECONDS,
        # `mixed` holds an int64 here and a double in `coercions`: the same
        # property name with two numeric types across entities of one kind.
        "mixed": 7,
    },
    {
        "id": "coercions",
        "big": 0,
        "small": -1,
        "ratio": float("nan"),
        "when": FAR_FUTURE_SECONDS,
        "mixed": 7.5,
    },
    {
        "id": "infinities",
        "big": 1,
        "small": -2,
        "ratio": float("inf"),
        "when": 0,
        "mixed": 0,
    },
    {
        "id": "negative-infinity",
        "big": 2,
        "small": -3,
        "ratio": float("-inf"),
        "when": TS_SECONDS,
        "mixed": -0.5,
    },
]

# Two levels of nesting; the base fixtures' `address` map is only one deep.
NESTED = {
    "outer": "yes",
    "inner": {"depth": 2, "leaf": "bottom"},
}


def _onestore_nested() -> bytes:
    inner = onestore_entity_ns(
        [("Values", "nested"), ("Inner", 1)],
        [
            onestore_property("depth", pv_int(NESTED["inner"]["depth"])),
            onestore_property("leaf", pv_string(NESTED["inner"]["leaf"]), MEANING_TEXT),
        ],
    )
    return onestore_entity_ns(
        [("Values", "nested"), ("Outer", 1)],
        [
            onestore_property("inner", pv_string(inner), MEANING_ENTITY_PROTO),
            onestore_property("outer", pv_string(NESTED["outer"]), MEANING_TEXT),
        ],
    )


def _onestore_number(value: int | float) -> bytes:
    return pv_double(value) if isinstance(value, float) else pv_int(value)


def onestore_value(row: dict) -> bytes:
    props = [
        onestore_property("big", pv_int(row["big"])),
        onestore_property("small", pv_int(row["small"])),
        onestore_property("ratio", pv_double(row["ratio"])),
        # GD_WHEN carries microseconds since the epoch, negative before 1970.
        onestore_property("when", pv_int(row["when"] * 1_000_000), MEANING_GD_WHEN),
        onestore_property("mixed", _onestore_number(row["mixed"])),
        onestore_property("nested", pv_string(_onestore_nested()), MEANING_ENTITY_PROTO),
    ]
    # A heterogeneous array: repeated Property entries sharing one name. Real
    # Datastore arrays are not type-homogeneous.
    props.extend(
        [
            onestore_property("grab_bag", pv_string("text"), MEANING_TEXT, multiple=True),
            onestore_property("grab_bag", pv_int(11), MEANING_NO_MEANING, multiple=True),
            onestore_property("grab_bag", pv_bool(True), MEANING_NO_MEANING, multiple=True),
        ]
    )
    if row["id"] == "limits":
        props.append(onestore_property("emptied", PV_NULL, MEANING_EMPTY_LIST))
    # `raw_property` (field 15) carries unindexed properties. A decoder that
    # reads only field 14 drops them without a word.
    raw = [onestore_property("unindexed_note", pv_string(f"raw:{row['id']}"), MEANING_TEXT)]
    return onestore_entity_ns([("Values", row["id"])], props, raw_properties=raw)


def _v1_number(value: int | float) -> bytes:
    return v1_double(value) if isinstance(value, float) else v1_int(value)


def v1_value(row: dict) -> bytes:
    return v1_entity_ns(
        [("Values", row["id"])],
        {
            "big": v1_int(row["big"]),
            "small": v1_int(row["small"]),
            "ratio": v1_double(row["ratio"]),
            "when": v1_timestamp(row["when"]),
            "mixed": _v1_number(row["mixed"]),
            "nested": v1_entity_value(
                {
                    "inner": v1_entity_value(
                        {
                            "depth": v1_int(NESTED["inner"]["depth"]),
                            "leaf": v1_string(NESTED["inner"]["leaf"]),
                        }
                    ),
                    "outer": v1_string(NESTED["outer"]),
                }
            ),
            "grab_bag": v1_array([v1_string("text"), v1_int(11), v1_bool(True)]),
            # `datastore.v1.Entity` has no unindexed/indexed distinction, so the
            # property that `export-edge-onestore` puts in `raw_property` is an
            # ordinary property here. Both trees therefore expose the same
            # field set, and only the legacy tree exercises field 15.
            "unindexed_note": v1_string(f"raw:{row['id']}"),
        },
    )


# --------------------------------------------------------------------------
# Awkward — property names that break generated DDL
# --------------------------------------------------------------------------

AWKWARD = [
    {"id": "first", "select": "reserved word", "group": 1, "flag": True},
    {"id": "second", "select": "also reserved", "group": 2, "flag": False},
]


def _awkward_props(row: dict) -> dict[str, object]:
    """Property name -> value, shared by both payload encodings."""
    return {
        # PostgreSQL reserved words; unquoted emission is a syntax error.
        "select": row["select"],
        "group": row["group"],
        # Case-only difference: folding to lower case merges these two.
        "Flag": row["flag"],
        "flag": not row["flag"],
        # A dot is how Datastore map keys get flattened, so a literal dot in a
        # property name is ambiguous with a nested field path.
        "meta.source": "literal-dot",
        # Leading digit: a bare `2fa_enabled` identifier does not parse.
        "2fa_enabled": True,
        # Multi-byte name — 5 characters but 15 bytes.
        "名前かなカナ": row["id"],
        # Longer than 63 bytes and identical to its sibling for the first 63.
        LONG_NAME_A: f"{row['id']}-a",
        LONG_NAME_B: f"{row['id']}-b",
    }


def onestore_awkward(row: dict) -> bytes:
    props = []
    for name, value in _awkward_props(row).items():
        if isinstance(value, bool):
            props.append(onestore_property(name, pv_bool(value)))
        elif isinstance(value, int):
            props.append(onestore_property(name, pv_int(value)))
        else:
            props.append(onestore_property(name, pv_string(value), MEANING_TEXT))
    return onestore_entity_ns([("Awkward", row["id"])], props)


def v1_awkward(row: dict) -> bytes:
    props = {}
    for name, value in _awkward_props(row).items():
        if isinstance(value, bool):
            props[name] = v1_bool(value)
        elif isinstance(value, int):
            props[name] = v1_int(value)
        else:
            props[name] = v1_string(value)
    return v1_entity_ns([("Awkward", row["id"])], props)


# --------------------------------------------------------------------------
# Scoped — the same key names in two non-default namespaces
# --------------------------------------------------------------------------

NAMESPACES = ["tenant-a", "tenant-b"]
SCOPED_IDS = ["shared-name", "also-shared"]


def onestore_scoped(namespace: str, ident: str) -> bytes:
    return onestore_entity_ns(
        [("Scoped", ident)],
        [
            onestore_property("tenant", pv_string(namespace), MEANING_TEXT),
            onestore_property("owner", pv_string(f"{namespace}/{ident}"), MEANING_TEXT),
        ],
        namespace=namespace,
    )


def v1_scoped(namespace: str, ident: str) -> bytes:
    return v1_entity_ns(
        [("Scoped", ident)],
        {"tenant": v1_string(namespace), "owner": v1_string(f"{namespace}/{ident}")},
        namespace=namespace,
    )


# --------------------------------------------------------------------------
# export metadata + writer
# --------------------------------------------------------------------------

EXPORT_TIMESTAMP = "2026-05-02T11:04:17_88421"


def overall_export_metadata(partitions: list[tuple[str, str]]) -> bytes:
    """`partitions` is a list of (partition_dir, kind) pairs."""
    inputs = b"".join(
        f_string(1, f"{partition}/kind_{kind}/{partition}_kind_{kind}.export_metadata")
        for partition, kind in partitions
    )
    body = f_bytes(1, inputs) + f_string(2, "datastore_v3") + f_string(3, APP_ID)
    return frame_records([body])


def kind_export_metadata(partition: str, kind: str) -> bytes:
    body = (
        f_bytes(1, f_string(1, f"{partition}/kind_{kind}/output-"))
        + f_string(2, "datastore_v3")
        + f_bytes(3, f_string(2, kind))
    )
    return frame_records([body])


def write_export(root: str, shards: dict[tuple[str, str, int], list[bytes]]) -> int:
    """`shards` maps (partition_dir, kind, shard_number) -> record payloads."""
    os.makedirs(root, exist_ok=True)
    partitions = sorted({(partition, kind) for partition, kind, _ in shards})
    with open(os.path.join(root, f"{EXPORT_TIMESTAMP}.overall_export_metadata"), "wb") as handle:
        handle.write(overall_export_metadata(partitions))

    for partition, kind in partitions:
        kind_dir = os.path.join(root, partition, f"kind_{kind}")
        os.makedirs(kind_dir, exist_ok=True)
        meta_name = f"{partition}_kind_{kind}.export_metadata"
        with open(os.path.join(kind_dir, meta_name), "wb") as handle:
            handle.write(kind_export_metadata(partition, kind))

    total = 0
    for (partition, kind, shard), payloads in sorted(shards.items()):
        path = os.path.join(root, partition, f"kind_{kind}", f"output-{shard}")
        with open(path, "wb") as handle:
            handle.write(frame_records(payloads))
        total += len(payloads)
    return total


def onestore_shards() -> dict[tuple[str, str, int], list[bytes]]:
    shards: dict[tuple[str, str, int], list[bytes]] = {}
    for row in SHARDS:
        shards.setdefault(("all_namespaces", "Shards", row["shard"]), []).append(
            onestore_shard(row)
        )
    shards[("all_namespaces", "Fragments", 0)] = onestore_fragments()
    shards[("all_namespaces", "Idents", 0)] = [onestore_ident(r) for r in IDENTS]
    shards[("all_namespaces", "Values", 0)] = [onestore_value(r) for r in VALUES]
    shards[("all_namespaces", "Awkward", 0)] = [onestore_awkward(r) for r in AWKWARD]
    for namespace in NAMESPACES:
        shards[(f"namespace_{namespace}", "Scoped", 0)] = [
            onestore_scoped(namespace, ident) for ident in SCOPED_IDS
        ]
    return shards


def v1_shards() -> dict[tuple[str, str, int], list[bytes]]:
    shards: dict[tuple[str, str, int], list[bytes]] = {}
    for row in SHARDS:
        shards.setdefault(("all_namespaces", "Shards", row["shard"]), []).append(v1_shard(row))
    shards[("all_namespaces", "Fragments", 0)] = v1_fragments()
    shards[("all_namespaces", "Idents", 0)] = [v1_ident(r) for r in IDENTS]
    shards[("all_namespaces", "Values", 0)] = [v1_value(r) for r in VALUES]
    shards[("all_namespaces", "Awkward", 0)] = [v1_awkward(r) for r in AWKWARD]
    for namespace in NAMESPACES:
        shards[(f"namespace_{namespace}", "Scoped", 0)] = [
            v1_scoped(namespace, ident) for ident in SCOPED_IDS
        ]
    return shards


EXPECTED_ENTITIES = (
    len(SHARDS)
    + 3  # Fragments
    + len(IDENTS)
    + len(VALUES)
    + len(AWKWARD)
    + len(NAMESPACES) * len(SCOPED_IDS)
)


def main() -> int:
    out_dir = sys.argv[1] if len(sys.argv) > 1 else os.path.dirname(os.path.abspath(__file__))

    for name, builder in (
        ("export-edge-onestore", onestore_shards),
        ("export-edge-v1", v1_shards),
    ):
        total = write_export(os.path.join(out_dir, name), builder())
        assert total == EXPECTED_ENTITIES, f"{name}: wrote {total}, expected {EXPECTED_ENTITIES}"
        print(f"  {name}/  {total} entities")

    print(f"wrote edge-case fixtures under {out_dir}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
