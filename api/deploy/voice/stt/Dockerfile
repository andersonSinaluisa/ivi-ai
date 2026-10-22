# jarvis-stt: faster-whisper en CPU para el servicio de voz del escritorio. No es parte del Cloud Core.
FROM python:3.12-slim
ENV PYTHONDONTWRITEBYTECODE=1 PYTHONUNBUFFERED=1 PIP_NO_CACHE_DIR=1 \
    WHISPER_MODEL=base WHISPER_CACHE=/models HF_HOME=/models/hf
WORKDIR /app
COPY requirements.txt .
RUN pip install --no-cache-dir -r requirements.txt \
 && useradd --uid 10001 --create-home --shell /usr/sbin/nologin stt \
 && install -d -o stt -g stt /models
COPY server.py .
USER stt
EXPOSE 9100
HEALTHCHECK --interval=30s --timeout=5s --start-period=120s --retries=3 \
  CMD python -c "import json,urllib.request;d=json.load(urllib.request.urlopen('http://127.0.0.1:9100/health',timeout=4));exit(0 if d['ready'] else 1)"
CMD ["uvicorn", "server:app", "--host", "127.0.0.1", "--port", "9100", "--workers", "1", "--no-server-header"]
