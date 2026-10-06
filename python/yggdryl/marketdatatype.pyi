"""Typing for `yggdryl.marketdatatype`: the members are the native table's,
listed for static checkers; `python/tests/test_marketdatatype.py` holds the
list to it."""

from __future__ import annotations

import enum
from typing import Literal, TypeAlias

from ._common import MetadataInput
from ._typing import TypedField
from .marketdatakind import MarketDataKind

class MarketDataType(enum.IntEnum):
    UKNW = 0
    ORDMKT = 101
    ORDLIMIT = 102
    ORDSTOP = 103
    ORDSTOPLIMIT = 104
    ORDMOC = 105
    ORDWOW = 106
    ORDLOB = 107
    ORDLWOW = 108
    ORDBASIS = 109
    ORDONCLOSE = 110
    ORDLOC = 111
    ORDFXMKT = 112
    ORDPREVQUOTED = 113
    ORDPREVINDIC = 114
    ORDFXLIMIT = 115
    ORDFXSWAP = 116
    ORDFXPREVQUOTED = 117
    ORDFUNARI = 118
    ORDMIT = 119
    ORDMKTLIMIT = 120
    ORDPREVFUND = 121
    ORDNEXTFUND = 122
    ORDPEGGED = 123
    ORDCOUNTER = 124
    ORDSTOPBO = 125
    ORDSTOPLIMITBO = 126
    ORDMKTBAND = 127
    ORDOTHER = 199
    QUOINDIC = 200
    QUOTRAD = 201
    QUORESTR = 202
    QUOCOUNTER = 203
    QUOINIT = 204
    QUOOTHER = 299
    TRDREG = 300
    TRDBLOCK = 301
    TRDEFP = 302
    TRDTRANSFER = 303
    TRDLATE = 304
    TRDT = 305
    TRDWAP = 306
    TRDBUNCHED = 307
    TRDLATEBUNCHED = 308
    TRDPRIORREF = 309
    TRDAFTERHOURS = 310
    TRDEFR = 311
    TRDEFS = 312
    TRDTAS = 315
    TRDAON = 316
    TRDERROR = 324
    TRDLARGE = 338
    TRDEXERCISE = 345
    TRDPORTFOLIO = 350
    TRDVWAP = 351
    TRDOTC = 354
    TRDOPENING = 356
    TRDNETTED = 357
    TRDDARK = 362
    TRDTECHNICAL = 363
    TRDBENCHMARK = 364
    TRDPACKAGE = 365
    TRDROLL = 366
    TRDCLOSING = 367
    TRDOTHER = 399
    BOOKBID = 400
    BOOKOFFER = 401
    BOOKTRADE = 402
    BOOKINDEX = 403
    BOOKOPEN = 404
    BOOKCLOSE = 405
    BOOKSETTLE = 406
    BOOKHIGH = 407
    BOOKLOW = 408
    BOOKVWAP = 409
    BOOKIMBALANCE = 410
    BOOKVOLUME = 411
    BOOKOI = 412
    BOOKMID = 417
    BOOKEMPTY = 418
    BOOKOTHER = 499
    TRPTSUBMIT = 500
    TRPTALLEGED = 501
    TRPTACCEPT = 502
    TRPTDECLINE = 503
    TRPTADDENDUM = 504
    TRPTNOWAS = 505
    TRPTCANCEL = 506
    TRPTBREAK = 507
    TRPTDEFAULTED = 508
    TRPTINVALIDCMTA = 509
    TRPTPENDED = 510
    TRPTALLEGEDNEW = 511
    TRPTALLEGEDADD = 512
    TRPTALLEGEDNOWAS = 513
    TRPTALLEGEDCANCEL = 514
    TRPTALLEGEDBREAK = 515
    TRPTOTHER = 599
    QRQMANUAL = 601
    QRQAUTO = 602
    QRQOTHER = 699
    MCXSECURITY = 701
    MCXUNDERLYING = 702
    MCXPRODUCT = 703
    MCXCFI = 704
    MCXSECTYPE = 705
    MCXSESSION = 706
    MCXALL = 707
    MCXMARKET = 708
    MCXSEGMENT = 709
    MCXGROUP = 710
    MCXISSUER = 711
    MCXUNDISSUER = 712
    MCXOTHER = 799
    MDRSNAPSHOT = 800
    MDRSUBSCRIBE = 801
    MDRUNSUBSCRIBE = 802
    MDROTHER = 899
    @property
    def description(self) -> str: ...
    @classmethod
    def from_spelling(cls, spelling: str) -> MarketDataType | None: ...
    @classmethod
    def from_fix(cls, tag: int, wire: str) -> MarketDataType | None: ...
    @property
    def fix_code(self) -> tuple[int, str] | None: ...
    @staticmethod
    def fix_tags(kind: MarketDataKind) -> tuple[int, ...]: ...
    @staticmethod
    def fix_tags_of(msgtype: str, kind: MarketDataKind) -> tuple[int, ...]: ...

MarketDataTypeField: TypeAlias = TypedField[Literal["marketdatatype"], MarketDataType]

def marketdatatype(
    name: str, *, nullable: bool = True, metadata: MetadataInput = None
) -> MarketDataTypeField: ...

__all__ = ["MarketDataType", "MarketDataTypeField", "marketdatatype"]
