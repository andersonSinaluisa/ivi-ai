"""``python -m app``: arranca el servicio en 127.0.0.1."""

from __future__ import annotations

import logging

import uvicorn

from .main import build_components, create_app


def main() -> None:
    logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)s %(name)s: %(message)s")
    components = build_components()
    app = create_app(components)
    s = components.settings
    uvicorn.run(app, host=s.host, port=s.port, log_level="info", ws_max_size=64 * 1024)


if __name__ == "__main__":
    main()
