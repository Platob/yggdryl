from __future__ import annotations

import copy
import pickle
from pathlib import Path

import pytest

from yggdryl import MediaType, MimeType


KNOWN_MIME_TYPES = {
    "OCTET_STREAM": "application/octet-stream",
    "JSON": "application/json",
    "JSON_LINES": "application/x-ndjson",
    "YAML": "application/yaml",
    "TOML": "application/toml",
    "CSV": "text/csv",
    "TSV": "text/tab-separated-values",
    "PARQUET": "application/vnd.apache.parquet",
    "ARROW_FILE": "application/vnd.apache.arrow.file",
    "ARROW_STREAM": "application/vnd.apache.arrow.stream",
    "AVRO": "application/avro",
    "ORC": "application/vnd.apache.orc",
    "PUFFIN": "application/vnd.apache.puffin",
    "PLAIN_TEXT": "text/plain",
    "ULLINK": "text/ullink",
    "FIX": "text/fix",
    "FIXUL": "text/fixul",
    "FIXML": "text/fixml",
    "MARKDOWN": "text/markdown",
    "HTML": "text/html",
    "CSS": "text/css",
    "JAVASCRIPT": "text/javascript",
    "XML": "application/xml",
    "PDF": "application/pdf",
    "CBOR": "application/cbor",
    "MESSAGE_PACK": "application/vnd.msgpack",
    "PROTOBUF": "application/protobuf",
    "SQLITE3": "application/vnd.sqlite3",
    "PNG": "image/png",
    "JPEG": "image/jpeg",
    "GIF": "image/gif",
    "WEBP": "image/webp",
    "SVG": "image/svg+xml",
    "MP3": "audio/mpeg",
    "WAV": "audio/wav",
    "OGG": "audio/ogg",
    "FLAC": "audio/flac",
    "MP4": "video/mp4",
    "WEBM": "video/webm",
    "WOFF": "font/woff",
    "WOFF2": "font/woff2",
    "TTF": "font/ttf",
    "OTF": "font/otf",
    "XLS": "application/vnd.ms-excel",
    "XLSX": "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
    "ODS": "application/vnd.oasis.opendocument.spreadsheet",
    "DOC": "application/msword",
    "DOCX": "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    "GZIP": "application/gzip",
    "ZSTD": "application/zstd",
    "BROTLI": "application/x-brotli",
    "ZLIB": "application/zlib",
    "COMPRESS": "application/x-compress",
    "BZIP2": "application/x-bzip2",
    "XZ": "application/x-xz",
    "LZ4": "application/x-lz4",
    "SNAPPY": "application/x-snappy-framed",
    "ZIP": "application/zip",
    "SEVEN_ZIP": "application/x-7z-compressed",
    "RAR": "application/vnd.rar",
    "TAR": "application/x-tar",
}


def test_mime_type_complete_known_constants_and_default() -> None:
    assert len(KNOWN_MIME_TYPES) == 60
    assert MimeType() == MimeType.OCTET_STREAM
    values = []
    for name, canonical in KNOWN_MIME_TYPES.items():
        value = getattr(MimeType, name)
        assert type(value) is MimeType
        assert str(value) == canonical
        assert value.is_known()
        assert MimeType.from_str(canonical) == value
        values.append(value)
    assert len(set(values)) == len(values)


def test_io_identity_is_derived_from_the_unencoded_mime_value() -> None:
    assert MimeType.CSV.is_io()
    assert MediaType.from_parts(MimeType.CSV, [MimeType.GZIP]).is_io()
    directory = MimeType("inode/directory")
    assert not directory.is_io()
    assert not MediaType(directory).is_io()


def test_mime_type_native_parsing_views_and_value_protocols(tmp_path: Path) -> None:
    custom = MimeType("Application/Vnd.Example+JSON")
    assert str(custom) == "application/vnd.example+json"
    assert custom.top_level == "application"
    assert custom.subtype == "vnd.example+json"
    assert custom.structured_suffix == "json"
    assert custom.extension == "json"
    assert custom.content_coding is None
    assert custom.format == "json"
    assert custom.is_structured()

    assert MimeType.from_extension(".json") == MimeType.JSON
    assert MimeType.from_extension(".puffin") == MimeType.PUFFIN
    assert MimeType.PUFFIN.extension == "puffin"
    assert MimeType.PUFFIN.is_binary()
    assert MimeType.PUFFIN.is_structured()
    assert not MimeType.PUFFIN.is_tabular()
    assert MimeType.from_path(tmp_path / "events.csv") == MimeType.CSV
    assert MimeType.from_content_type('Application/JSON; charset="utf-8"') == MimeType.JSON
    assert MimeType.from_content_coding("gzip") == MimeType.GZIP
    assert MimeType.GZIP.content_coding == "gzip"
    assert MimeType.JSON.format == "json"
    assert MimeType.JSON == copy.copy(MimeType.JSON)
    assert MimeType.JSON == copy.deepcopy(MimeType.JSON)
    assert MimeType.JSON == pickle.loads(pickle.dumps(MimeType.JSON))
    assert MimeType.from_json(MimeType.JSON.into_json()) == MimeType.JSON
    assert hash(MimeType.JSON) == hash(MimeType.from_value(MimeType.JSON))
    assert MimeType.JSON.stable_hash() == MimeType.from_str("application/json").stable_hash()

    with pytest.raises(ValueError):
        MimeType.from_content_type("application/json; charset")
    with pytest.raises(ValueError):
        MimeType.from_content_coding("identity")
    with pytest.raises(TypeError):
        MimeType.from_value(object())


def test_media_type_default_compound_headers_and_detached_snapshots() -> None:
    assert MediaType().base == MimeType.OCTET_STREAM
    assert MediaType().encodings == ()
    media = MediaType.from_parts(MimeType.CSV, (value for value in ["gzip", "zstd"]))
    assert str(media) == "text/csv;encodings=application/gzip,application/zstd"
    assert media.base == MimeType.CSV
    assert media.encodings == (MimeType.GZIP, MimeType.ZSTD)
    assert media.encoding == MimeType.ZSTD
    assert media.extensions == ["csv", "gz", "zst"]
    assert list(media) == [MimeType.GZIP, MimeType.ZSTD]
    assert media[-1] == MimeType.ZSTD
    assert MimeType.GZIP in media
    assert len(media) == 2

    detached = media.encodings
    iterator = iter(media)
    media.clear_encodings()
    assert detached == (MimeType.GZIP, MimeType.ZSTD)
    assert list(iterator) == [MimeType.GZIP, MimeType.ZSTD]
    assert media.encodings == ()

    from_headers = MediaType.from_content_headers(
        "Application/JSON; Charset=utf-8", " gzip ,\tbr, compress "
    )
    assert from_headers.base == MimeType.JSON
    assert from_headers.encodings == (
        MimeType.GZIP,
        MimeType.BROTLI,
        MimeType.COMPRESS,
    )
    assert MediaType.from_content_headers().base == MimeType.OCTET_STREAM
    assert MediaType.from_path("events.json.gz").encodings == (MimeType.GZIP,)
    assert MediaType.from_file_name("events.csv.zst").base == MimeType.CSV
    assert MediaType.from_extension("json").base == MimeType.JSON
    assert MediaType.from_extensions(value for value in ["json", "gz"]) == MediaType.from_parts(
        MimeType.JSON, [MimeType.GZIP]
    )
    relative = MediaType.from_str("folder/orders.csv.gz")
    assert relative.base == MimeType.CSV
    assert relative.encodings == (MimeType.GZIP,)
    assert MediaType.from_str("application/vnd.example.report+json").base == MimeType(
        "application/vnd.example.report+json"
    )
    assert MediaType.from_extension(" .TBZ2\t") == MediaType.from_parts(
        MimeType.TAR, [MimeType.BZIP2]
    )


def test_media_type_mutation_is_atomic_hash_locked_and_round_trips() -> None:
    media = MediaType.from_parts(MimeType.JSON, [MimeType.GZIP])
    before = str(media)

    def invalid_values() -> object:
        yield MimeType.ZSTD
        yield object()

    with pytest.raises(TypeError):
        media.set_encodings(invalid_values())
    assert str(media) == before
    with pytest.raises(ValueError):
        media.push_encoding(MimeType.ZIP)
    assert str(media) == before
    with pytest.raises(TypeError):
        media.set_base(object())
    assert str(media) == before
    media.set_base(MimeType.CSV)
    media.set_encodings(value for value in [MimeType.GZIP, MimeType.ZSTD])
    media.push_encoding(MimeType.BROTLI)
    assert media.base == MimeType.CSV
    assert media.encodings == (MimeType.GZIP, MimeType.ZSTD, MimeType.BROTLI)
    assert MediaType.from_json(media.into_json()) == media
    assert copy.copy(media) == media
    assert copy.deepcopy(media) == media
    assert pickle.loads(pickle.dumps(media)) == media
    assert media.stable_hash() == MediaType.from_str(str(media)).stable_hash()
    assert hash(media) == hash(MediaType.from_str(str(media)))

    keyed = {media: "stable"}
    with pytest.raises(TypeError, match="hashed MediaType is frozen"):
        media.set_base(MimeType.JSON)
    with pytest.raises(TypeError, match="hashed MediaType is frozen"):
        media.clear_encodings()
    assert keyed[media] == "stable"

    copied = copy.copy(media)
    copied.set_base(MimeType.JSON)
    assert copied != media
    restored = pickle.loads(pickle.dumps(media))
    restored.clear_encodings()
    assert restored != media

    with pytest.raises(TypeError, match="iterable"):
        MediaType.from_parts(MimeType.JSON, "gzip")
    with pytest.raises(TypeError, match="iterable"):
        MediaType(media).set_encodings("gzip")
    with pytest.raises(ValueError):
        MediaType.from_content_headers(None, "identity")
