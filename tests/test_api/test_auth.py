"""Tests for authentication utilities."""

from app.api.auth import (
    create_session_token,
    generate_link_token,
    hash_password,
    validate_link_token,
    validate_session_token,
    verify_password,
)


class TestPasswordHashing:
    def test_hash_and_verify(self):
        hashed = hash_password("mysecretpassword")
        assert verify_password("mysecretpassword", hashed)

    def test_wrong_password_fails(self):
        hashed = hash_password("correct")
        assert not verify_password("wrong", hashed)

    def test_hash_is_not_plaintext(self):
        hashed = hash_password("test")
        assert hashed != "test"
        assert hashed.startswith("$2b$")


class TestSessionTokens:
    def test_create_and_validate(self):
        token = create_session_token("user-123")
        user_id = validate_session_token(token)
        assert user_id == "user-123"

    def test_invalid_token(self):
        user_id = validate_session_token("garbage-token")
        assert user_id is None

    def test_token_is_string(self):
        token = create_session_token("user-123")
        assert isinstance(token, str)


class TestLinkTokens:
    def test_create_and_validate(self):
        token = generate_link_token("user-456")
        user_id = validate_link_token(token)
        assert user_id == "user-456"

    def test_invalid_link_token(self):
        user_id = validate_link_token("bad-token")
        assert user_id is None

    def test_session_token_not_valid_as_link(self):
        # Session tokens don't have purpose=link
        token = create_session_token("user-123")
        user_id = validate_link_token(token)
        assert user_id is None
