from pydantic_settings import BaseSettings, SettingsConfigDict


class Settings(BaseSettings):
    model_config = SettingsConfigDict(env_file=".env", env_file_encoding="utf-8", extra="ignore")

    # Application
    app_name: str = "Mika"
    debug: bool = False

    # Anthropic
    anthropic_api_key: str = ""
    claude_sonnet_model: str = "claude-sonnet-4-5-20250929"
    claude_opus_model: str = "claude-opus-4-6-20260205"

    # Telegram
    telegram_bot_token: str = ""
    telegram_webhook_url: str = ""

    # WhatsApp (Phase 3)
    whatsapp_phone_id: str = ""
    whatsapp_token: str = ""
    whatsapp_verify_token: str = ""

    # Neo4j
    neo4j_uri: str = "bolt://localhost:7687"
    neo4j_user: str = "neo4j"
    neo4j_password: str = "password"

    # PostgreSQL
    database_url: str = "postgresql+asyncpg://mika:mika@localhost:5432/mika"

    # Redis
    redis_url: str = "redis://localhost:6379/0"

    # Celery
    celery_broker_url: str = "redis://localhost:6379/1"
    celery_result_backend: str = "redis://localhost:6379/2"

    # Encryption
    encryption_key: str = ""

    # Web search
    tavily_api_key: str = ""

    # Google Calendar (Phase 4)
    google_client_id: str = ""
    google_client_secret: str = ""

    # Server
    host: str = "0.0.0.0"
    port: int = 8000


settings = Settings()
