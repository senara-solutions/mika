"""Authentication routes: signup, login, logout."""

import uuid

from fastapi import APIRouter, Form, Request
from fastapi.responses import HTMLResponse, RedirectResponse
from sqlalchemy import select

from app.api.auth import (
    create_session_token,
    generate_link_token,
    hash_password,
    verify_password,
)
from app.common.db import async_session_factory
from app.common.logging import get_logger
from app.models.consent import UserConsent
from app.models.user import User

logger = get_logger(__name__)

router = APIRouter(tags=["auth"])

SESSION_COOKIE = "mika_session"


@router.post("/signup")
async def signup(
    request: Request,
    email: str = Form(...),
    password: str = Form(...),
):
    """Create a new user account."""
    async with async_session_factory() as session:
        # Check if email already exists
        result = await session.execute(
            select(User).where(User.email == email)
        )
        if result.scalar_one_or_none():
            return request.app.state.templates.TemplateResponse(
                "auth/signup.html",
                {"request": request, "error": "Email already registered"},
            )

        user = User(
            id=uuid.uuid4(),
            email=email,
            password_hash=hash_password(password),
            onboarding_state="completed",
            onboarding_completed=True,
        )
        session.add(user)
        session.add(UserConsent(user_id=user.id))
        await session.commit()

        token = create_session_token(str(user.id))
        response = RedirectResponse("/dashboard", status_code=303)
        response.set_cookie(
            SESSION_COOKIE, token, httponly=True, max_age=86400 * 30
        )
        return response


@router.post("/login")
async def login(
    request: Request,
    email: str = Form(...),
    password: str = Form(...),
):
    """Log in with email and password."""
    async with async_session_factory() as session:
        result = await session.execute(
            select(User).where(User.email == email)
        )
        user = result.scalar_one_or_none()

        if not user or not user.password_hash:
            return request.app.state.templates.TemplateResponse(
                "auth/login.html",
                {"request": request, "error": "Invalid email or password"},
            )

        if not verify_password(password, user.password_hash):
            return request.app.state.templates.TemplateResponse(
                "auth/login.html",
                {"request": request, "error": "Invalid email or password"},
            )

        token = create_session_token(str(user.id))
        response = RedirectResponse("/dashboard", status_code=303)
        response.set_cookie(
            SESSION_COOKIE, token, httponly=True, max_age=86400 * 30
        )
        return response


@router.get("/logout")
async def logout():
    """Log out and clear session."""
    response = RedirectResponse("/login", status_code=303)
    response.delete_cookie(SESSION_COOKIE)
    return response


@router.get("/signup", response_class=HTMLResponse)
async def signup_page(request: Request):
    return request.app.state.templates.TemplateResponse(
        "auth/signup.html", {"request": request}
    )


@router.get("/login", response_class=HTMLResponse)
async def login_page(request: Request):
    return request.app.state.templates.TemplateResponse(
        "auth/login.html", {"request": request}
    )


@router.get("/link-telegram")
async def link_telegram(request: Request):
    """Generate a Telegram linking URL."""
    user = getattr(request.state, "user", None)
    if not user:
        return RedirectResponse("/login", status_code=303)

    token = generate_link_token(str(user.id))
    bot_username = "MikaBot"  # TODO: get from config
    link_url = f"https://t.me/{bot_username}?start=link_{token}"

    return request.app.state.templates.TemplateResponse(
        "dashboard/link_telegram.html",
        {"request": request, "user": user, "link_url": link_url},
    )
