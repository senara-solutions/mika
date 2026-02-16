"""Fernet field encryption for PII in Postgres."""

from cryptography.fernet import Fernet

from app.config import settings

_fernet: Fernet | None = None


def _get_fernet() -> Fernet:
    global _fernet
    if _fernet is None:
        if not settings.encryption_key:
            raise RuntimeError(
                "ENCRYPTION_KEY not set. Generate with: "
                "Fernet.generate_key().decode()"
            )
        _fernet = Fernet(settings.encryption_key.encode())
    return _fernet


def encrypt(plaintext: str) -> bytes:
    return _get_fernet().encrypt(plaintext.encode())


def decrypt(ciphertext: bytes) -> str:
    return _get_fernet().decrypt(ciphertext).decode()
