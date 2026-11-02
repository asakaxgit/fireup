#!/usr/bin/env python3
"""Generate realistic Firestore/Datastore managed-export fixtures for fireup.

Two fixture trees are produced so that LevelDB *framing* bugs can be told apart
from *payload proto* bugs:

  export-onestore/   LevelDB log framing + legacy `storage_onestore_v3.EntityProto`
                     payloads. This is what a real `gcloud datastore export` /
                     `gcloud firestore export` actually writes.

  export-v1-entity/  Identical LevelDB log framing, but payloads are
                     `google.datastore.v1.Entity` messages — the shape
                     src/leveldb_parser/protobuf_entity.rs currently decodes.

Both trees mimic the real managed-export directory layout:

  <root>/<timestamp>.overall_export_metadata
  <root>/all_namespaces/kind_<Kind>/all_namespaces_kind_<Kind>.export_metadata
  <root>/all_namespaces/kind_<Kind>/output-0

Framing follows the LevelDB log format: 32 KiB blocks, 7-byte record headers
(4-byte masked CRC32C of type+payload, 2-byte little-endian length, 1-byte
record type), zero padding for trailing block fragments shorter than a header.

Usage:
    python3 tests/fixtures/generate_datastore_export.py [output-dir] [--crc ieee]

`--crc ieee` emits record checksums the way fireup currently computes them
(unmasked zlib/IEEE CRC-32 over type+payload) instead of the spec's masked
CRC32C. That variant is a *diagnostic* only — it is bug-compatible, so write it
to a scratch directory and never commit it as a fixture.
"""

from __future__ import annotations

import os
import struct
import sys
import zlib

# --------------------------------------------------------------------------
# protobuf wire primitives
# --------------------------------------------------------------------------

WIRE_VARINT = 0
WIRE_FIXED64 = 1
WIRE_LEN = 2
WIRE_START_GROUP = 3
WIRE_END_GROUP = 4


def varint(value: int) -> bytes:
    if value < 0:
        # proto2 int64 negatives are encoded as 10-byte two's complement varints
        value += 1 << 64
    out = bytearray()
    while True:
        byte = value & 0x7F
        value >>= 7
        if value:
            out.append(byte | 0x80)
        else:
            out.append(byte)
            return bytes(out)


def tag(field: int, wire: int) -> bytes:
    return varint((field << 3) | wire)


def f_varint(field: int, value: int) -> bytes:
    return tag(field, WIRE_VARINT) + varint(value)


def f_bool(field: int, value: bool) -> bytes:
    return f_varint(field, 1 if value else 0)


def f_double(field: int, value: float) -> bytes:
    return tag(field, WIRE_FIXED64) + struct.pack("<d", value)


def f_bytes(field: int, value: bytes) -> bytes:
    return tag(field, WIRE_LEN) + varint(len(value)) + value


def f_string(field: int, value: str) -> bytes:
    return f_bytes(field, value.encode("utf-8"))


def f_group(field: int, payload: bytes) -> bytes:
    return tag(field, WIRE_START_GROUP) + payload + tag(field, WIRE_END_GROUP)


# --------------------------------------------------------------------------
# LevelDB log framing (leveldb/db/log_format.h)
# --------------------------------------------------------------------------

BLOCK_SIZE = 32768
HEADER_SIZE = 7

RECORD_FULL = 1
RECORD_FIRST = 2
RECORD_MIDDLE = 3
RECORD_LAST = 4

CRC32C_POLY = 0x82F63B78
_CRC32C_TABLE: list[int] = []
for _i in range(256):
    _c = _i
    for _ in range(8):
        _c = (_c >> 1) ^ (CRC32C_POLY if _c & 1 else 0)
    _CRC32C_TABLE.append(_c)


def crc32c(data: bytes, crc: int = 0) -> int:
    crc ^= 0xFFFFFFFF
    for byte in data:
        crc = _CRC32C_TABLE[(crc ^ byte) & 0xFF] ^ (crc >> 8)
    return crc ^ 0xFFFFFFFF


def mask_crc(crc: int) -> int:
    """LevelDB masks CRCs so a stored CRC is never the CRC of real data."""
    return (((crc >> 15) | (crc << 17)) + 0xA282EAD8) & 0xFFFFFFFF


def record_checksum(record_type: int, body: bytes, algorithm: str) -> int:
    if algorithm == "crc32c":
        return mask_crc(crc32c(bytes([record_type]) + body))
    if algorithm == "ieee":
        # Bug-compatible with fireup's current crc32fast implementation.
        return zlib.crc32(bytes([record_type]) + body) & 0xFFFFFFFF
    raise ValueError(f"unknown checksum algorithm: {algorithm}")


def frame_records(payloads: list[bytes], algorithm: str = "crc32c") -> bytes:
    """Pack payloads into LevelDB log blocks, fragmenting across 32 KiB blocks."""
    out = bytearray()
    for payload in payloads:
        view = memoryview(payload)
        first = True
        while True:
            room = BLOCK_SIZE - (len(out) % BLOCK_SIZE)
            if room < HEADER_SIZE:
                out.extend(b"\x00" * room)
                room = BLOCK_SIZE
            capacity = room - HEADER_SIZE
            chunk = view[:capacity]
            view = view[capacity:]
            last = len(view) == 0
            if first and last:
                record_type = RECORD_FULL
            elif first:
                record_type = RECORD_FIRST
            elif last:
                record_type = RECORD_LAST
            else:
                record_type = RECORD_MIDDLE
            body = bytes(chunk)
            crc = record_checksum(record_type, body, algorithm)
            out.extend(struct.pack("<IHB", crc, len(body), record_type))
            out.extend(body)
            first = False
            if last:
                break
    return bytes(out)


# --------------------------------------------------------------------------
# legacy storage_onestore_v3.EntityProto  (what real exports contain)
# --------------------------------------------------------------------------

# Property.Meaning values used below
MEANING_NO_MEANING = 0
MEANING_GD_WHEN = 7
MEANING_GEORSS_POINT = 9
MEANING_BLOB = 14
MEANING_TEXT = 15
MEANING_ENTITY_PROTO = 19
MEANING_EMPTY_LIST = 24

APP_ID = "s~fireup-fixture"


def onestore_path(elements: list[tuple[str, object]]) -> bytes:
    """Path { repeated group Element = 1 { type = 2; id = 3; name = 4; } }"""
    out = bytearray()
    for kind, ident in elements:
        element = bytearray(f_string(2, kind))
        if isinstance(ident, int):
            element += f_varint(3, ident)
        else:
            element += f_string(4, str(ident))
        out += f_group(1, bytes(element))
    return bytes(out)


def onestore_reference(elements: list[tuple[str, object]], namespace: str = "") -> bytes:
    """Reference { app = 13; name_space = 20; path = 14; }"""
    out = bytearray(f_string(13, APP_ID))
    out += f_bytes(14, onestore_path(elements))
    if namespace:
        out += f_string(20, namespace)
    return bytes(out)


def pv_int(value: int) -> bytes:
    return f_varint(1, value)


def pv_bool(value: bool) -> bytes:
    return f_bool(2, value)


def pv_string(value: bytes | str) -> bytes:
    raw = value.encode("utf-8") if isinstance(value, str) else value
    return f_bytes(3, raw)


def pv_double(value: float) -> bytes:
    return f_double(4, value)


def pv_point(x: float, y: float) -> bytes:
    """PropertyValue { optional group PointValue = 5 { x = 6; y = 7; } }"""
    return f_group(5, f_double(6, x) + f_double(7, y))


def pv_reference(elements: list[tuple[str, object]]) -> bytes:
    """PropertyValue.ReferenceValue group 12 { app = 13; PathElement group 14 {...} }"""
    body = bytearray(f_string(13, APP_ID))
    for kind, ident in elements:
        element = bytearray(f_string(15, kind))
        if isinstance(ident, int):
            element += f_varint(16, ident)
        else:
            element += f_string(17, str(ident))
        body += f_group(14, bytes(element))
    return f_group(12, bytes(body))


PV_NULL = b""  # PropertyValue with no field set == null


def onestore_property(
    name: str,
    value: bytes,
    meaning: int = MEANING_NO_MEANING,
    multiple: bool = False,
) -> bytes:
    """Property { meaning = 1; name = 3; multiple = 4; value = 5; }"""
    out = bytearray()
    if meaning != MEANING_NO_MEANING:
        out += f_varint(1, meaning)
    out += f_string(3, name)
    out += f_bool(4, multiple)
    out += f_bytes(5, value)
    return bytes(out)


def onestore_entity(
    elements: list[tuple[str, object]],
    properties: list[bytes],
    raw_properties: list[bytes] | None = None,
) -> bytes:
    """EntityProto { key = 13; property = 14; raw_property = 15; entity_group = 16; }"""
    out = bytearray()
    out += f_bytes(13, onestore_reference(elements))
    out += f_bytes(16, onestore_path(elements[:1]))
    for prop in properties:
        out += f_bytes(14, prop)
    for prop in raw_properties or []:
        out += f_bytes(15, prop)
    return bytes(out)


# --------------------------------------------------------------------------
# google.datastore.v1.Entity  (what fireup currently decodes)
# --------------------------------------------------------------------------


def v1_key(elements: list[tuple[str, object]], project: str = "fireup-fixture") -> bytes:
    """Key { partition_id = 1; repeated PathElement path = 2; }"""
    out = bytearray(f_bytes(1, f_string(2, project)))
    for kind, ident in elements:
        element = bytearray(f_string(1, kind))
        if isinstance(ident, int):
            element += f_varint(2, ident)
        else:
            element += f_string(3, str(ident))
        out += f_bytes(2, bytes(element))
    return bytes(out)


def v1_null() -> bytes:
    return f_varint(11, 0)


def v1_bool(value: bool) -> bytes:
    return f_bool(1, value)


def v1_int(value: int) -> bytes:
    return f_varint(2, value)


def v1_double(value: float) -> bytes:
    return f_double(3, value)


def v1_timestamp(seconds: int, nanos: int = 0) -> bytes:
    inner = f_varint(1, seconds) + (f_varint(2, nanos) if nanos else b"")
    return f_bytes(10, inner)


def v1_string(value: str) -> bytes:
    return f_string(17, value)


def v1_blob(value: bytes) -> bytes:
    return f_bytes(18, value)


def v1_key_value(elements: list[tuple[str, object]]) -> bytes:
    return f_bytes(5, v1_key(elements))


def v1_entity_value(properties: dict[str, bytes]) -> bytes:
    return f_bytes(6, v1_entity(None, properties))


def v1_array(values: list[bytes]) -> bytes:
    return f_bytes(9, b"".join(f_bytes(1, v) for v in values))


def v1_entity(
    elements: list[tuple[str, object]] | None,
    properties: dict[str, bytes],
) -> bytes:
    """Entity { key = 1; map<string, Value> properties = 3; }"""
    out = bytearray()
    if elements:
        out += f_bytes(1, v1_key(elements))
    for name in sorted(properties):
        entry = f_string(1, name) + f_bytes(2, properties[name])
        out += f_bytes(3, entry)
    return bytes(out)


# --------------------------------------------------------------------------
# fixture content — one dataset expressed in both payload encodings
# --------------------------------------------------------------------------

# 2026-03-14T09:26:53.589793Z
TS_SECONDS = 1773480413
TS_MICROS = TS_SECONDS * 1_000_000 + 589793

AVATAR_BLOB = bytes(range(256)) * 3  # 768 bytes of non-UTF8 binary
BIG_BLOB = bytes((i * 31 + 7) & 0xFF for i in range(45000))  # forces fragmentation

USERS = [
    {
        "id": "ada",
        "display_name": "Ada Lovelace",
        "email": "ada@example.com",
        "age": 36,
        "score": 99.5,
        "active": True,
        "bio": "Mathematician. Wrote the first algorithm intended for a machine.",
        "nickname": None,
        "created_at": (TS_SECONDS, 589793000),
        "avatar": AVATAR_BLOB,
        "tags": ["math", "computing"],
        "address": {"city": "London", "postcode": "W1J 9BR", "verified": True},
        "location": (51.5074, -0.1278),
        "manager": None,
    },
    {
        "id": "grace",
        "display_name": "Grace Hopper",
        "email": "grace@example.com",
        "age": 45,
        "score": 98.25,
        "active": True,
        "bio": "Compiler pioneer.",
        "nickname": "Amazing Grace",
        "created_at": (TS_SECONDS - 86400, 0),
        "avatar": AVATAR_BLOB[:64],
        "tags": ["compilers", "navy", "cobol"],
        "address": {"city": "New York", "postcode": "10001", "verified": False},
        "location": (40.7128, -74.0060),
        "manager": [("Users", "ada")],
    },
    {
        "id": 4503599627370496,  # numeric (auto-allocated) key id
        "display_name": "Anonymous",
        "email": "anon@example.com",
        "age": 0,
        "score": 0.0,
        "active": False,
        "bio": "",
        "nickname": None,
        "created_at": (TS_SECONDS - 3600, 1),
        "avatar": b"",
        "tags": [],
        "address": {"city": "", "postcode": "", "verified": False},
        "location": (0.0, 0.0),
        "manager": None,
    },
    {
        "id": "linus",
        "display_name": "Linus Torvalds",
        "email": "linus@example.com",
        "age": 56,
        "score": -12.0,  # negative double
        "active": True,
        "bio": "Kernel.",
        "nickname": None,
        "created_at": (TS_SECONDS - 604800, 250000000),
        "avatar": BIG_BLOB,  # > 32 KiB: record must fragment across blocks
        "tags": ["kernel", "git"],
        "address": {"city": "Portland", "postcode": "97201", "verified": True},
        "location": (45.5152, -122.6784),
        "manager": [("Users", "grace")],
    },
    {
        "id": "unicode-ユーザー",
        "display_name": "テスト ユーザー",
        "email": "test@example.jp",
        "age": -1,  # negative int64
        "score": 50.0,
        "active": False,
        "bio": "絵文字 🔥 included",
        "nickname": None,
        "created_at": (TS_SECONDS + 7200, 0),
        "avatar": AVATAR_BLOB[:16],
        "tags": ["日本語"],
        "address": {"city": "東京", "postcode": "100-0001", "verified": True},
        "location": (35.6762, 139.6503),
        "manager": None,
    },
]

ORDERS = [
    {"id": 1001, "user": "ada", "total_cents": 12999, "currency": "GBP", "shipped": True, "items": 3},
    {"id": 1002, "user": "ada", "total_cents": 500, "currency": "GBP", "shipped": False, "items": 1},
    {"id": 1003, "user": "grace", "total_cents": 7250, "currency": "USD", "shipped": True, "items": 2},
    {"id": 1004, "user": "grace", "total_cents": 0, "currency": "USD", "shipped": False, "items": 0},
    {"id": 1005, "user": "linus", "total_cents": 999999, "currency": "EUR", "shipped": True, "items": 42},
    {"id": 1006, "user": "linus", "total_cents": 1, "currency": "EUR", "shipped": True, "items": 1},
    {"id": 1007, "user": "unicode-ユーザー", "total_cents": 250000, "currency": "JPY", "shipped": False, "items": 5},
]


def onestore_user(user: dict) -> bytes:
    props = [
        onestore_property("active", pv_bool(user["active"])),
        onestore_property("address", pv_string(
            onestore_entity(
                [("Users", user["id"]), ("Address", 1)],
                [
                    onestore_property("city", pv_string(user["address"]["city"]), MEANING_TEXT),
                    onestore_property("postcode", pv_string(user["address"]["postcode"])),
                    onestore_property("verified", pv_bool(user["address"]["verified"])),
                ],
            )
        ), MEANING_ENTITY_PROTO),
        onestore_property("age", pv_int(user["age"])),
        onestore_property("avatar", pv_string(user["avatar"]), MEANING_BLOB),
        onestore_property("bio", pv_string(user["bio"]), MEANING_TEXT),
        onestore_property("created_at", pv_int(TS_MICROS), MEANING_GD_WHEN),
        onestore_property("display_name", pv_string(user["display_name"])),
        onestore_property("email", pv_string(user["email"])),
        onestore_property(
            "location",
            pv_point(user["location"][0], user["location"][1]),
            MEANING_GEORSS_POINT,
        ),
        onestore_property("nickname", PV_NULL if user["nickname"] is None else pv_string(user["nickname"])),
        onestore_property("score", pv_double(user["score"])),
    ]
    if user["manager"]:
        props.append(onestore_property("manager", pv_reference(user["manager"])))
    if user["tags"]:
        props.extend(
            onestore_property("tags", pv_string(tag_value), MEANING_TEXT, multiple=True)
            for tag_value in user["tags"]
        )
    else:
        props.append(onestore_property("tags", PV_NULL, MEANING_EMPTY_LIST))
    props.sort(key=lambda raw: raw)  # exports emit properties name-ordered
    return onestore_entity([("Users", user["id"])], props)


def onestore_order(order: dict) -> bytes:
    props = [
        onestore_property("currency", pv_string(order["currency"])),
        onestore_property("items", pv_int(order["items"])),
        onestore_property("placed_at", pv_int(TS_MICROS), MEANING_GD_WHEN),
        onestore_property("shipped", pv_bool(order["shipped"])),
        onestore_property("total_cents", pv_int(order["total_cents"])),
        onestore_property("user", pv_reference([("Users", order["user"])])),
    ]
    return onestore_entity([("Users", order["user"]), ("Orders", order["id"])], props)


def v1_user(user: dict) -> bytes:
    props = {
        "active": v1_bool(user["active"]),
        "address": v1_entity_value(
            {
                "city": v1_string(user["address"]["city"]),
                "postcode": v1_string(user["address"]["postcode"]),
                "verified": v1_bool(user["address"]["verified"]),
            }
        ),
        "age": v1_int(user["age"]),
        "avatar": v1_blob(user["avatar"]),
        "bio": v1_string(user["bio"]),
        "created_at": v1_timestamp(*user["created_at"]),
        "display_name": v1_string(user["display_name"]),
        "email": v1_string(user["email"]),
        "nickname": v1_null() if user["nickname"] is None else v1_string(user["nickname"]),
        "score": v1_double(user["score"]),
        "tags": v1_array([v1_string(t) for t in user["tags"]]),
    }
    if user["manager"]:
        props["manager"] = v1_key_value(user["manager"])
    return v1_entity([("Users", user["id"])], props)


def v1_order(order: dict) -> bytes:
    return v1_entity(
        [("Users", order["user"]), ("Orders", order["id"])],
        {
            "currency": v1_string(order["currency"]),
            "items": v1_int(order["items"]),
            "placed_at": v1_timestamp(TS_SECONDS),
            "shipped": v1_bool(order["shipped"]),
            "total_cents": v1_int(order["total_cents"]),
            "user": v1_key_value([("Users", order["user"])]),
        },
    )


# --------------------------------------------------------------------------
# export metadata files
# --------------------------------------------------------------------------

EXPORT_TIMESTAMP = "2026-03-14T09:26:53_12345"


def overall_export_metadata(kinds: list[str], algorithm: str) -> bytes:
    """ExportMetadata-shaped record; real exports LevelDB-frame this too."""
    inputs = b"".join(
        f_string(1, f"all_namespaces/kind_{kind}/all_namespaces_kind_{kind}.export_metadata")
        for kind in kinds
    )
    body = f_bytes(1, inputs) + f_string(2, "datastore_v3") + f_string(3, APP_ID)
    return frame_records([body], algorithm)


def kind_export_metadata(kind: str, algorithm: str) -> bytes:
    body = (
        f_bytes(1, f_string(1, f"all_namespaces/kind_{kind}/output-"))
        + f_string(2, "datastore_v3")
        + f_bytes(3, f_string(2, kind))
    )
    return frame_records([body], algorithm)


# --------------------------------------------------------------------------
# writer
# --------------------------------------------------------------------------


def write_export(root: str, payloads_by_kind: dict[str, list[bytes]], algorithm: str) -> None:
    os.makedirs(root, exist_ok=True)
    kinds = list(payloads_by_kind)
    with open(os.path.join(root, f"{EXPORT_TIMESTAMP}.overall_export_metadata"), "wb") as handle:
        handle.write(overall_export_metadata(kinds, algorithm))
    for kind, payloads in payloads_by_kind.items():
        kind_dir = os.path.join(root, "all_namespaces", f"kind_{kind}")
        os.makedirs(kind_dir, exist_ok=True)
        meta_name = f"all_namespaces_kind_{kind}.export_metadata"
        with open(os.path.join(kind_dir, meta_name), "wb") as handle:
            handle.write(kind_export_metadata(kind, algorithm))
        with open(os.path.join(kind_dir, "output-0"), "wb") as handle:
            handle.write(frame_records(payloads, algorithm))


def main() -> int:
    argv = sys.argv[1:]
    algorithm = "crc32c"
    if "--crc" in argv:
        index = argv.index("--crc")
        algorithm = argv[index + 1]
        del argv[index : index + 2]
    out_dir = argv[0] if argv else os.path.dirname(os.path.abspath(__file__))
    suffix = "" if algorithm == "crc32c" else f"-{algorithm}crc"

    write_export(
        os.path.join(out_dir, f"export-onestore{suffix}"),
        {
            "Users": [onestore_user(u) for u in USERS],
            "Orders": [onestore_order(o) for o in ORDERS],
        },
        algorithm,
    )
    write_export(
        os.path.join(out_dir, f"export-v1-entity{suffix}"),
        {
            "Users": [v1_user(u) for u in USERS],
            "Orders": [v1_order(o) for o in ORDERS],
        },
        algorithm,
    )

    print(f"wrote fixtures under {out_dir} (checksum={algorithm})")
    print(f"  export-onestore{suffix}/   {len(USERS)} Users + {len(ORDERS)} Orders (legacy EntityProto)")
    print(f"  export-v1-entity{suffix}/  {len(USERS)} Users + {len(ORDERS)} Orders (datastore.v1.Entity)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
