# jarvis-tts: voz con Piper en CPU para el servicio de voz del escritorio. No es parte del Cloud Core.
FROM python:3.12-slim
ENV PYTHONDONTWRITEBYTECODE=1 PYTHONUNBUFFERED=1 PIP_NO_CACHE_DIR=1 \
    PIPER_VOICE=es_MX-claude-high PIPER_DIR=/models
WORKDIR /app
COPY requirements.txt .
RUN pip install --no-cache-dir -r requirements.txt \
 && useradd --uid 10002 --create-home --shell /usr/sbin/nologin tts \
 && install -d -o tts -g tts /models
COPY server.py .
USER tts
EXPOSE 9200
HEALTHCHECK --interval=30s --timeout=5s --start-period=120s --retries=3 \
  CMD python -c "import json,urllib.request;d=json.load(urllib.request.urlopen('http://127.0.0.1:9200/health',timeout=4));exit(0 if d['ready'] else 1)"
CMD ["uvicorn", "server:app", "--host", "127.0.0.1", "--port", "9200", "--workers", "1", "--no-server-header"]
