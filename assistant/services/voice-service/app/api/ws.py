"""WebSocket de eventos y control.

Protocolo (packages/shared-types/src/voice.ts):
1. El cliente se conecta y en menos de 5 s envía ``{"type": "auth", "token": "…"}``.
   El token no viaja en la URL para que no quede en registros.
2. El servidor responde ``hello`` y empieza a enviar eventos (estado, nivel, transcripción…).
3. El cliente envía comandos: ptt_start, ptt_stop, cancel, speak, stop_speaking, …
"""

from __future__ import annotations

import asyncio
import contextlib
import json
import logging

from fastapi import APIRouter, WebSocket, WebSocketDisconnect

from ..config.security import token_matches

log = logging.getLogger(__name__)
ws_router = APIRouter()

CLOSE_UNAUTHORIZED = 4401
CLOSE_FORBIDDEN_ORIGIN = 4403
AUTH_TIMEOUT_S = 5
MAX_MESSAGE = 16 * 1024


@ws_router.websocket("/ws")
async def ws_endpoint(ws: WebSocket) -> None:
    c = ws.app.state.c
    origin = ws.headers.get("origin")
    if origin and origin not in c.settings.allowed_origins:
        await ws.close(code=CLOSE_FORBIDDEN_ORIGIN)
        return
    await ws.accept()
    try:
        first = await asyncio.wait_for(ws.receive_text(), timeout=AUTH_TIMEOUT_S)
        msg = json.loads(first)
    except (asyncio.TimeoutError, json.JSONDecodeError, WebSocketDisconnect):
        with contextlib.suppress(Exception):
            await ws.close(code=CLOSE_UNAUTHORIZED)
        return
    if msg.get("type") != "auth" or not token_matches(c.token, msg.get("token")):
        await ws.close(code=CLOSE_UNAUTHORIZED)
        return

    engine = c.engine
    queue = c.bus.subscribe()
    await ws.send_json(engine.snapshot())

    async def pump() -> None:
        while True:
            event = await queue.get()
            await ws.send_json(event)

    sender = asyncio.create_task(pump())
    try:
        while True:
            text = await ws.receive_text()
            if len(text) > MAX_MESSAGE:
                continue
            try:
                cmd = json.loads(text)
            except json.JSONDecodeError:
                continue
            await dispatch(engine, cmd)
    except WebSocketDisconnect:
        pass
    finally:
        sender.cancel()
        c.bus.unsubscribe(queue)


async def dispatch(engine, cmd: dict) -> None:  # noqa: ANN001
    kind = cmd.get("type")
    if kind == "ptt_start":
        await engine.ptt_start()
    elif kind == "ptt_stop":
        await engine.ptt_stop()
    elif kind == "cancel":
        await engine.cancel()
    elif kind == "speak":
        await engine.speak(str(cmd.get("text", "")))
    elif kind == "stop_speaking":
        await engine.stop_speaking()
    elif kind == "thinking":
        await engine.thinking()
    elif kind == "agent_done":
        await engine.agent_done()
    elif kind == "set_listening":
        engine.set_listening(bool(cmd.get("enabled")))
    else:
        log.debug("comando desconocido: %s", kind)
