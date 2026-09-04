"""``Timezone`` canonicalizes what Python spells three different ways."""

from __future__ import annotations

import copy
import datetime
import pickle
import zoneinfo

import pytest

from yggdryl import Timezone


def utc(year: int, month: int, day: int, hour: int = 0) -> int:
    """Epoch seconds for a UTC civil moment."""
    moment = datetime.datetime(year, month, day, hour, tzinfo=datetime.timezone.utc)
    return int(moment.timestamp())


class TestInference:
    """Every way Python names a zone arrives at the same value."""

    def test_a_name_an_alias_and_a_zoneinfo_are_one_zone(self) -> None:
        assert Timezone("Asia/Calcutta") == Timezone("Asia/Kolkata")
        assert Timezone(zoneinfo.ZoneInfo("Asia/Calcutta")) == Timezone("Asia/Kolkata")
        # Python itself does not consider these equal, which is the point.
        assert zoneinfo.ZoneInfo("Asia/Calcutta") != zoneinfo.ZoneInfo("Asia/Kolkata")

    def test_a_fixed_offset_tzinfo_is_accepted(self) -> None:
        offset = datetime.timezone(datetime.timedelta(hours=5, minutes=30))

        assert Timezone(offset) == Timezone("+05:30")
        assert Timezone(datetime.timezone.utc) == Timezone.UTC

    def test_every_spelling_of_utc_is_utc(self) -> None:
        for value in ("UTC", "utc", "Z", "GMT", "Etc/UTC", "+00:00"):
            assert Timezone(value) == Timezone.UTC
            assert Timezone(value).is_utc()

    def test_a_bad_name_is_refused(self) -> None:
        with pytest.raises(ValueError):
            Timezone("")
        with pytest.raises(ValueError):
            Timezone("+25:00")
        with pytest.raises(TypeError):
            Timezone(object())

    def test_the_key_attribute_matches_zoneinfo(self) -> None:
        # Named `key` so a Timezone can stand in wherever only the name is read.
        assert Timezone("US/Eastern").key == "America/New_York"
        assert Timezone(zoneinfo.ZoneInfo("Europe/Paris")).key == "Europe/Paris"


class TestOffsets:
    """The rules answer the way the IANA database does."""

    def test_a_northern_zone_switches_in_march_and_november(self) -> None:
        new_york = Timezone("America/New_York")

        assert new_york.offset_at(utc(2024, 1, 15)) == -5 * 3600
        assert new_york.offset_at(utc(2024, 7, 15)) == -4 * 3600
        assert new_york.abbreviation_at(utc(2024, 1, 15)) == "EST"
        assert new_york.abbreviation_at(utc(2024, 7, 15)) == "EDT"
        assert new_york.is_saving_at(utc(2024, 7, 15))
        assert not new_york.is_saving_at(utc(2024, 1, 15))

    def test_explicit_local_and_utc_conversions_use_into_names(self) -> None:
        instant = utc(2024, 1, 15, 12)
        assert Timezone.UTC.into_local(instant) == instant
        assert Timezone.UTC.into_utc(instant) == instant

    def test_a_southern_zone_is_saving_across_the_new_year(self) -> None:
        sydney = Timezone("Australia/Sydney")

        assert sydney.offset_at(utc(2024, 1, 15)) == 11 * 3600
        assert sydney.offset_at(utc(2024, 7, 15)) == 10 * 3600

    def test_the_offsets_agree_with_zoneinfo(self) -> None:
        # The registry is hand-rolled, so check it against the system database
        # for the zones and instants this build claims to know.
        for name in (
            "America/New_York",
            "America/Los_Angeles",
            "Europe/Paris",
            "Europe/London",
            "Australia/Sydney",
            "Asia/Tokyo",
            "Asia/Kolkata",
            "Pacific/Auckland",
        ):
            zone = Timezone(name)
            system = zoneinfo.ZoneInfo(name)
            for month in (1, 4, 7, 10):
                moment = datetime.datetime(2024, month, 15, 12, tzinfo=system)
                expected = int(moment.utcoffset().total_seconds())
                assert zone.offset_at(int(moment.timestamp())) == expected, (
                    f"{name} in month {month}"
                )

    def test_a_zone_without_rules_declines_to_answer(self) -> None:
        unknown = Timezone("Custom/Accepted")

        assert unknown.key == "Custom/Accepted"
        assert not unknown.is_known()
        assert unknown.offset_at(0) is None
        assert unknown.standard_offset is None
        # Refusing is recoverable; a plausible wrong offset is not.
        with pytest.raises(ValueError):
            unknown.into_local(0)

    def test_utcoffset_duck_types_as_a_tzinfo(self) -> None:
        paris = Timezone("Europe/Paris")

        assert paris.utcoffset(utc(2024, 7, 15)) == datetime.timedelta(hours=2)
        assert paris.utcoffset(utc(2024, 1, 15)) == datetime.timedelta(hours=1)
        assert Timezone("Custom/Accepted").utcoffset(0) is None


class TestRegistry:
    """The installed set needs no files, environment, or network."""

    def test_the_common_zones_are_installed(self) -> None:
        registered = {zone.key for zone in Timezone.registered()}

        assert len(registered) > 60
        assert {"UTC", "Europe/Paris", "America/New_York", "Asia/Tokyo"} <= registered
        # Every registered name parses back to itself.
        for zone in Timezone.registered():
            assert Timezone(zone.key) == zone

    def test_aliases_resolve_to_registered_zones(self) -> None:
        registered = {zone.key for zone in Timezone.registered()}

        for alias, canonical in Timezone.aliases():
            assert Timezone(alias).key == canonical
            # Asia/Jerusalem is deliberately not in the rules table, so an
            # alias may canonicalize to a name with no rules.
            if canonical in registered:
                assert Timezone(alias).is_known()


class TestProtocols:
    """The value behaves like the rest of the binding's value types."""

    def test_it_hashes_compares_and_sorts(self) -> None:
        assert hash(Timezone("US/Eastern")) == hash(Timezone("America/New_York"))
        assert Timezone("Europe/Paris") != Timezone("Europe/Berlin")
        assert Timezone("UTC") != "UTC"
        assert Timezone("UTC") != zoneinfo.ZoneInfo("UTC")
        assert sorted([Timezone("UTC"), Timezone("Europe/Paris")])[0].key == "Europe/Paris"
        assert {Timezone("Z"), Timezone("UTC")} == {Timezone.UTC}

    def test_it_round_trips_through_pickle_and_copy(self) -> None:
        zone = Timezone("Australia/Sydney")

        assert pickle.loads(pickle.dumps(zone)) == zone
        assert copy.copy(zone) == zone
        assert copy.deepcopy(zone) == zone

    def test_it_prints_its_canonical_name(self) -> None:
        assert str(Timezone("US/Pacific")) == "America/Los_Angeles"
        assert repr(Timezone("UTC")) == 'Timezone("UTC")'
