"""FastAPI middleware for session-based authentication."""

import uuid

from sqlalchemy import select
from starlette.middleware.base import BaseHTTPMiddleware, RequestResponseEndpoint
from starlette.requests import Request
from starlette.responses import Response

from app.api.auth import validate_session_token
from app.api.routes.auth import SESSION_COOKIE
from app.common.db import async_session_factory
from app.models.user import User


class SessionAuthMiddleware(BaseHTTPMiddleware):
    """Load user from session cookie into request.state.user."""

    EXCLUDED_PATHS = {
        "/health",
        "/webhook/telegram",
        "/webhook/whatsapp",
        "/login",
        "/signup",
        "/logout",
    }

    async def dispatch(
        self, request: Request, call_next: RequestResponseEndpoint
    ) -> Response:
        request.state.user = None

        # Skip auth for excluded paths and static files
        path = request.url.path
        if path in self.EXCLUDED_PATHS or path.startswith("/static"):
            return await call_next(request)

        token = request.cookies.get(SESSION_COOKIE)
        if token:
            user_id = validate_session_token(token)
            if user_id:
                try:
                    async with async_session_factory() as session:
                        result = await session.execute(
                            select(User).where(
                                User.id == uuid.UUID(user_id)
                            )
                        )
                        request.state.user = result.scalar_one_or_none()
                except Exception:
                    pass

        return await call_next(request)
