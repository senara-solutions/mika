"""Authentication utilities: password hashing, session management."""

import bcrypt
from itsdangerous import BadSignature, SignatureExpired, URLSafeTimedSerializer

from app.config import settings

SESSION_MAX_AGE = 86400 * 30  # 30 days


def _get_serializer() -> URLSafeTimedSerializer:
    secret = settings.encryption_key or "dev-secret-change-me"
    return URLSafeTimedSerializer(secret)


def hash_password(password: str) -> str:
    return bcrypt.hashpw(
        password.encode("utf-8"), bcrypt.gensalt()
    ).decode("utf-8")


def verify_password(plain: str, hashed: str) -> bool:
    return bcrypt.checkpw(
        plain.encode("utf-8"), hashed.encode("utf-8")
    )


def create_session_token(user_id: str) -> str:
    """Create a signed session token."""
    s = _get_serializer()
    return s.dumps({"uid": user_id})


def validate_session_token(token: str) -> str | None:
    """Validate a session token. Returns user_id or None."""
    s = _get_serializer()
    try:
        data = s.loads(token, max_age=SESSION_MAX_AGE)
        return data.get("uid")
    except (BadSignature, SignatureExpired):
        return None


def generate_link_token(user_id: str) -> str:
    """Generate a token for Telegram account linking."""
    s = _get_serializer()
    return s.dumps({"uid": user_id, "purpose": "link"})


def validate_link_token(token: str) -> str | None:
    """Validate a link token. Returns user_id or None."""
    s = _get_serializer()
    try:
        data = s.loads(token, max_age=3600)  # 1 hour expiry
        if data.get("purpose") != "link":
            return None
        return data.get("uid")
    except (BadSignature, SignatureExpired):
        return None
