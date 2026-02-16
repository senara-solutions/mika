web: uvicorn app.api.main:app --host 0.0.0.0 --port ${PORT:-8000}
worker: celery -A app.worker.celery_app worker --loglevel=info --concurrency=2
beat: celery -A app.worker.celery_app beat --loglevel=info
