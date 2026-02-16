"""FastAPI application with Telegram webhook and web dashboard."""

from contextlib import asynccontextmanager
from pathlib import Path

from aiogram.types import Update
from fastapi import FastAPI, Request
from fastapi.responses import JSONResponse
from fastapi.templating import Jinja2Templates

from app.api.middleware import SessionAuthMiddleware
from app.api.routes.auth import router as auth_router
from app.api.routes.dashboard import router as dashboard_router
from app.channels.router import register_adapter
from app.channels.telegram import TelegramAdapter
from app.channels.telegram.bot import dp, get_bot
from app.channels.telegram.handlers import router as tg_router
from app.channels.telegram.middleware import (
    DbSessionMiddleware,
    TypingMiddleware,
)
from app.common.db import async_session_factory
from app.common.logging import get_logger, setup_logging
from app.config import settings

logger = get_logger(__name__)

TEMPLATES_DIR = Path(__file__).resolve().parent.parent / "templates"


@asynccontextmanager
async def lifespan(application: FastAPI):
    setup_logging("DEBUG" if settings.debug else "INFO")
    logger.info("Starting Mika API server")

    # Set up Jinja2 templates
    application.state.templates = Jinja2Templates(
        directory=str(TEMPLATES_DIR)
    )

    # Register channel adapters
    register_adapter("telegram", TelegramAdapter())

    # Set up Telegram dispatcher
    dp.include_router(tg_router)
    dp.message.middleware(TypingMiddleware())
    dp.message.middleware(DbSessionMiddleware(async_session_factory))

    bot = get_bot()

    # Set webhook if URL configured
    if settings.telegram_webhook_url:
        await bot.set_webhook(
            url=f"{settings.telegram_webhook_url}/webhook/telegram",
            drop_pending_updates=True,
        )

    yield

    # Cleanup
    if settings.telegram_webhook_url:
        await bot.delete_webhook()
    await bot.session.close()


app = FastAPI(title="Mika API", lifespan=lifespan)

# Add session auth middleware
app.add_middleware(SessionAuthMiddleware)

# Include routers
app.include_router(auth_router)
app.include_router(dashboard_router)


@app.get("/health")
async def health():
    return {"status": "ok"}


@app.get("/")
async def root():
    """Redirect to dashboard or login."""
    from fastapi.responses import RedirectResponse

    return RedirectResponse("/dashboard")


@app.post("/webhook/telegram")
async def telegram_webhook(request: Request):
    """Receive Telegram updates via webhook."""
    try:
        bot = get_bot()
        data = await request.json()
        update = Update.model_validate(data, context={"bot": bot})
        await dp.feed_update(bot=bot, update=update)
        return JSONResponse({"ok": True})
    except Exception:
        logger.exception("Error processing Telegram webhook")
        return JSONResponse({"ok": False}, status_code=500)
