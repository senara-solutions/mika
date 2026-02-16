"""Dashboard routes: settings, overview."""

from fastapi import APIRouter, Form, Request
from fastapi.responses import HTMLResponse, RedirectResponse
from sqlalchemy import select

from app.common.db import async_session_factory
from app.common.logging import get_logger
from app.models.user import User

logger = get_logger(__name__)

router = APIRouter(prefix="/dashboard", tags=["dashboard"])


@router.get("", response_class=HTMLResponse)
async def dashboard_home(request: Request):
    """Dashboard overview page."""
    user = getattr(request.state, "user", None)
    if not user:
        return RedirectResponse("/login", status_code=303)

    return request.app.state.templates.TemplateResponse(
        "dashboard/home.html",
        {"request": request, "user": user},
    )


@router.get("/settings", response_class=HTMLResponse)
async def settings_page(request: Request):
    """User settings page."""
    user = getattr(request.state, "user", None)
    if not user:
        return RedirectResponse("/login", status_code=303)

    return request.app.state.templates.TemplateResponse(
        "dashboard/settings.html",
        {"request": request, "user": user},
    )


@router.post("/settings")
async def update_settings(
    request: Request,
    timezone: str = Form("UTC"),
    preferred_channel: str = Form("telegram"),
):
    """Update user settings."""
    user = getattr(request.state, "user", None)
    if not user:
        return RedirectResponse("/login", status_code=303)

    async with async_session_factory() as session:
        result = await session.execute(
            select(User).where(User.id == user.id)
        )
        db_user = result.scalar_one_or_none()
        if db_user:
            db_user.timezone = timezone
            db_user.preferred_channel = preferred_channel
            await session.commit()

    return RedirectResponse("/dashboard/settings?saved=1", status_code=303)
