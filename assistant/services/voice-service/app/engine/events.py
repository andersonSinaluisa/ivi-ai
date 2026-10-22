"""Bus de eventos hacia los clientes WebSocket."""

from __future__ import annotations

import asyncio
from typing import Any


class EventBus:
    def __init__(self, maxsize: int = 256) -> None:
        self._subscribers: set[asyncio.Queue[dict[str, Any]]] = set()
        self._maxsize = maxsize
        self.history: list[dict[str, Any]] = []  # para pruebas y diagnóstico (acotado)

    def subscribe(self) -> asyncio.Queue[dict[str, Any]]:
        q: asyncio.Queue[dict[str, Any]] = asyncio.Queue(maxsize=self._maxsize)
        self._subscribers.add(q)
        return q

    def unsubscribe(self, q: asyncio.Queue[dict[str, Any]]) -> None:
        self._subscribers.discard(q)

    def publish(self, event: dict[str, Any]) -> None:
        if event.get("type") != "level":
            self.history.append(event)
            del self.history[:-200]
        for q in list(self._subscribers):
            if q.full():
                # Cliente lento: se descartan eventos viejos antes que bloquear el audio.
                try:
                    q.get_nowait()
                except asyncio.QueueEmpty:
                    pass
            q.put_nowait(event)
