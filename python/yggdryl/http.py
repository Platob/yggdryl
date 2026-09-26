"""HTTP backed entirely by the Rust core: a requests-shaped client, and a server.

``Session``, ``Request``, ``Response`` and ``Stream`` are the storage roles an
``http`` or ``https`` location takes, so every ``IOBase`` verb answers on them:
``IOBase("https://host/data.parquet")`` reads records with ranged ``GET``
requests, ``session / "users"`` is the ``Request`` for that resource, and a
``Response`` is its body as a handle. Beside the byte and record verbs they
speak the requests vocabulary - ``session.get(url, params=..., json=...)``,
``response.status_code``, ``response.json()``, ``raise_for_status`` - each a
redirect into the core. Every call that reaches the network releases the
interpreter, so threads share one ``Session`` and send side by side.

``Pages`` walks a paginated API one ``Response`` per page and lays the rows
out as Arrow; ``Server`` hosts ``IOBase`` handles and Python routes over
HTTP/1.1. The module-level verbs send on the process-wide default session,
the one ``session()`` answers.
"""

from __future__ import annotations

import datetime
from collections.abc import Iterable, Mapping

from ._native import (
    Client,
    Headers,
    Pages,
    Request,
    Response,
    Server,
    Session,
    Stream,
    Url,
    http_session as _http_session,
)

Pairs = Mapping[str, object] | Iterable[tuple[str, object]]
HeadersInput = Headers | Mapping[str, str] | Iterable[tuple[str, str]]
Auth = tuple[str, str] | str
Timeout = float | datetime.timedelta
Body = bytes | bytearray | memoryview | str | Pairs

__all__ = [
    "Client",
    "Headers",
    "Pages",
    "Request",
    "Response",
    "Server",
    "Session",
    "Stream",
    "delete",
    "get",
    "head",
    "patch",
    "post",
    "put",
    "session",
]


def session() -> Session:
    """The process-wide default session: shared options, one cookie jar."""
    return _http_session()


def get(
    url: str | Url,
    *,
    params: Pairs | None = None,
    headers: HeadersInput | None = None,
    auth: Auth | None = None,
    timeout: Timeout | None = None,
    allow_redirects: bool | None = None,
    stream: bool = False,
) -> Response:
    """``GET url`` on the default session."""
    return session().get(
        url,
        params=params,
        headers=headers,
        auth=auth,
        timeout=timeout,
        allow_redirects=allow_redirects,
        stream=stream,
    )


def head(
    url: str | Url,
    *,
    params: Pairs | None = None,
    headers: HeadersInput | None = None,
    auth: Auth | None = None,
    timeout: Timeout | None = None,
    allow_redirects: bool | None = None,
) -> Response:
    """``HEAD url`` on the default session."""
    return session().head(
        url,
        params=params,
        headers=headers,
        auth=auth,
        timeout=timeout,
        allow_redirects=allow_redirects,
    )


def post(
    url: str | Url,
    data: Body | None = None,
    *,
    json: object = None,
    params: Pairs | None = None,
    headers: HeadersInput | None = None,
    auth: Auth | None = None,
    timeout: Timeout | None = None,
) -> Response:
    """``POST`` a body to ``url`` on the default session."""
    return session().post(
        url,
        data,
        json=json,
        params=params,
        headers=headers,
        auth=auth,
        timeout=timeout,
    )


def put(
    url: str | Url,
    data: Body | None = None,
    *,
    json: object = None,
    params: Pairs | None = None,
    headers: HeadersInput | None = None,
    auth: Auth | None = None,
    timeout: Timeout | None = None,
) -> Response:
    """``PUT`` a body to ``url`` on the default session."""
    return session().put(
        url,
        data,
        json=json,
        params=params,
        headers=headers,
        auth=auth,
        timeout=timeout,
    )


def patch(
    url: str | Url,
    data: Body | None = None,
    *,
    json: object = None,
    params: Pairs | None = None,
    headers: HeadersInput | None = None,
    auth: Auth | None = None,
    timeout: Timeout | None = None,
) -> Response:
    """``PATCH`` a body to ``url`` on the default session."""
    return session().patch(
        url,
        data,
        json=json,
        params=params,
        headers=headers,
        auth=auth,
        timeout=timeout,
    )


def delete(
    url: str | Url,
    *,
    params: Pairs | None = None,
    headers: HeadersInput | None = None,
    auth: Auth | None = None,
    timeout: Timeout | None = None,
) -> Response:
    """``DELETE url`` on the default session."""
    return session().delete(
        url, params=params, headers=headers, auth=auth, timeout=timeout
    )
