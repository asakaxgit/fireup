#!/usr/bin/env python3
"""Independently verify the generated export fixtures.

This reader is deliberately written from the LevelDB log spec rather than
reusing the generator's code paths, so a passing run is evidence that the
fixture framing and payload protos are self-consistent. Any fireup failure
against these files is therefore a fireup finding, not a fixture artifact.

Usage:
    python3 tests/fixtures/verify_datastore_export.py [fixtures-dir]
"""

from __future__ import annotations

import os
import struct
import sys

BLOCK_SIZE = 32768
HEADER_SIZE = 7
RECORD_FULL, RECORD_FIRST, RECORD_MIDDLE, RECORD_LAST = 1, 2, 3, 4

_POLY = 0x82F63B78
_TABLE = []
for _i in range(256):
    _c = _i
    for _ in range(8):
        _c = (_c >> 1) ^ (_POLY if _c & 1 else 0)
    _TABLE.append(_c)


def crc32c(data: bytes) -> int:
    crc = 0xFFFFFFFF
    for byte in data:
        crc = _TABLE[(crc ^ byte) & 0xFF] ^ (crc >> 8)
    return crc ^ 0xFFFFFFFF


def mask_crc(crc: int) -> int:
    return (((crc >> 15) | (crc << 17)) + 0xA282EAD8) & 0xFFFFFFFF


def read_log(path: str) -> list[bytes]:
    """Unframe a LevelDB log file, verifying every masked CRC32C."""
    with open(path, "rb") as handle:
        raw = handle.read()

    records: list[bytes] = []
    pending = bytearray()
    for block_start in range(0, len(raw), BLOCK_SIZE):
        block = raw[block_start : block_start + BLOCK_SIZE]
        offset = 0
        while offset + HEADER_SIZE <= len(block):
            crc, length, record_type = struct.unpack_from("<IHB", block, offset)
            if (crc, length, record_type) == (0, 0, 0):
                break  # trailer padding
            body = block[offset + HEADER_SIZE : offset + HEADER_SIZE + length]
            if len(body) != length:
                raise AssertionError(f"{path}: truncated record at block {block_start}+{offset}")
            expected = mask_crc(crc32c(bytes([record_type]) + body))
            if expected != crc:
                raise AssertionError(
                    f"{path}: CRC mismatch at {block_start}+{offset}: "
                    f"stored 0x{crc:08x} computed 0x{expected:08x}"
                )
            if record_type == RECORD_FULL:
                assert not pending, f"{path}: FULL record while a fragment was open"
                records.append(body)
            elif record_type == RECORD_FIRST:
                assert not pending, f"{path}: FIRST record while a fragment was open"
                pending = bytearray(body)
            elif record_type in (RECORD_MIDDLE, RECORD_LAST):
                assert pending, f"{path}: {record_type} fragment without a FIRST"
                pending += body
                if record_type == RECORD_LAST:
                    records.append(bytes(pending))
                    pending = bytearray()
            else:
                raise AssertionError(f"{path}: invalid record type {record_type}")
            offset += HEADER_SIZE + length
    assert not pending, f"{path}: file ends mid-fragment"
    return records


def parse_fields(data: bytes) -> list[tuple[int, int, object]]:
    """Walk a protobuf message, returning (field, wire_type, value) triples.

    Groups (wire types 3/4) are returned with their raw inner bytes, which is
    the part prost cannot represent at all.
    """
    out: list[tuple[int, int, object]] = []
    pos = 0
    while pos < len(data):
        key, pos = read_varint(data, pos)
        field, wire = key >> 3, key & 7
        if wire == 0:
            value, pos = read_varint(data, pos)
        elif wire == 1:
            value, pos = data[pos : pos + 8], pos + 8
        elif wire == 2:
            length, pos = read_varint(data, pos)
            value, pos = data[pos : pos + length], pos + length
        elif wire == 5:
            value, pos = data[pos : pos + 4], pos + 4
        elif wire == 3:
            start = pos
            end = pos
            depth = 1
            while depth:
                before = pos
                inner_key, pos = read_varint(data, pos)
                inner_wire = inner_key & 7
                if inner_wire == 3:
                    depth += 1
                elif inner_wire == 4:
                    depth -= 1
                    if depth == 0:
                        end = before  # exclude the END_GROUP tag itself
                        break
                elif inner_wire == 0:
                    _, pos = read_varint(data, pos)
                elif inner_wire == 1:
                    pos += 8
                elif inner_wire == 2:
                    inner_len, pos = read_varint(data, pos)
                    pos += inner_len
                elif inner_wire == 5:
                    pos += 4
                else:
                    raise AssertionError(f"bad wire type {inner_wire} in group")
            value = data[start:end]
        elif wire == 4:
            raise AssertionError("unexpected END_GROUP")
        else:
            raise AssertionError(f"unsupported wire type {wire}")
        out.append((field, wire, value))
    return out


def read_varint(data: bytes, pos: int) -> tuple[int, int]:
    result = 0
    shift = 0
    while True:
        byte = data[pos]
        pos += 1
        result |= (byte & 0x7F) << shift
        if not byte & 0x80:
            return result, pos
        shift += 7


def first(fields, field_number, wire=2):
    for field, this_wire, value in fields:
        if field == field_number and this_wire == wire:
            return value
    return None


def onestore_identity(payload: bytes) -> tuple[str, str, int]:
    """Return (kind, id, property_count) for a legacy EntityProto record."""
    fields = parse_fields(payload)
    reference = first(fields, 13)
    assert reference is not None, "EntityProto has no key (field 13)"
    path = first(parse_fields(reference), 14)
    assert path is not None, "Reference has no path (field 14)"
    elements = [v for f, w, v in parse_fields(path) if f == 1 and w == 3]
    assert elements, "Path has no Element groups — proto2 group encoding expected"
    leaf = parse_fields(elements[-1])
    kind = first(leaf, 2).decode()
    name = first(leaf, 4)
    ident = name.decode() if name is not None else str(first(leaf, 3, wire=0))
    properties = [v for f, w, v in fields if f == 14 and w == 2]
    return kind, ident, len(properties)


def v1_identity(payload: bytes) -> tuple[str, str, int]:
    """Return (kind, id, property_count) for a datastore.v1.Entity record."""
    fields = parse_fields(payload)
    key = first(fields, 1)
    assert key is not None, "Entity has no key (field 1)"
    elements = [v for f, w, v in parse_fields(key) if f == 2 and w == 2]
    assert elements, "Key has no path elements"
    leaf = parse_fields(elements[-1])
    kind = first(leaf, 1).decode()
    name = first(leaf, 3)
    ident = name.decode() if name is not None else str(first(leaf, 2, wire=0))
    properties = [v for f, w, v in fields if f == 3 and w == 2]
    return kind, ident, len(properties)


def check_tree(root: str, identity) -> int:
    """Walk every partition (`all_namespaces/`, `namespace_<ns>/`) under `root`."""
    total = 0
    partitions = sorted(
        entry
        for entry in os.listdir(root)
        if os.path.isdir(os.path.join(root, entry))
        and (entry == "all_namespaces" or entry.startswith("namespace_"))
    )
    assert partitions, f"{root}: no all_namespaces/ or namespace_<ns>/ partition directory"
    for partition in partitions:
        kinds_dir = os.path.join(root, partition)
        for kind_entry in sorted(os.listdir(kinds_dir)):
            kind_dir = os.path.join(kinds_dir, kind_entry)
            # `output-10` must sort after `output-2`, so sort on the shard number.
            outputs = sorted(
                (f for f in os.listdir(kind_dir) if f.startswith("output-")),
                key=lambda name: int(name[len("output-") :]),
            )
            assert outputs, f"{kind_dir}: no output-N files"
            for output in outputs:
                path = os.path.join(kind_dir, output)
                records = read_log(path)
                assert records, f"{path}: no records"
                for record in records:
                    kind, ident, property_count = identity(record)
                    assert kind == kind_entry[len("kind_") :], f"{path}: kind {kind} != {kind_entry}"
                    assert ident, f"{path}: empty id"
                    assert property_count > 0, f"{path}: {kind}/{ident} has no properties"
                    total += 1
                print(f"  {os.path.relpath(path, root):<55} {len(records):>3} records, "
                      f"{os.path.getsize(path):>6} bytes")
    metadata = [f for f in os.listdir(root) if f.endswith(".overall_export_metadata")]
    assert len(metadata) == 1, f"{root}: expected exactly one overall_export_metadata"
    assert read_log(os.path.join(root, metadata[0])), "overall_export_metadata is empty"
    return total


# (tree, expected entity count) — the counts come from the generators.
TREES = [
    ("export-onestore", onestore_identity, 12),  # 5 Users + 7 Orders
    ("export-v1-entity", v1_identity, 12),
    ("export-edge-onestore", onestore_identity, 25),
    ("export-edge-v1", v1_identity, 25),
]


def main() -> int:
    base = sys.argv[1] if len(sys.argv) > 1 else os.path.dirname(os.path.abspath(__file__))
    for name, identity, expected in TREES:
        print(f"{name}:")
        total = check_tree(os.path.join(base, name), identity)
        assert total == expected, f"{name}: {total} entities, expected {expected}"
        print(f"  OK — {total} entities, all CRC32C checksums valid\n")
    print("fixtures verified")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
