"""Tests for WhatsApp webhook handlers."""

import uuid
from unittest.mock import AsyncMock, MagicMock, patch

import pytest

from app.channels.whatsapp.handlers import (
    _get_or_create_whatsapp_user,
    _handle_message,
)


def _make_wa_payload(text: str = "Hello", from_id: str = "2348012345678"):
    """Create a standard WhatsApp webhook payload."""
    return {
        "entry": [
            {
                "changes": [
                    {
                        "value": {
                            "messages": [
                                {
                                    "from": from_id,
                                    "type": "text",
                                    "text": {"body": text},
                                }
                            ]
                        }
                    }
                ]
            }
        ]
    }


class TestVerifyWebhook:
    @pytest.mark.asyncio
    async def test_valid_verification(self):
        from httpx import ASGITransport, AsyncClient

        from app.api.main import app

        transport = ASGITransport(app=app)
        async with AsyncClient(
            transport=transport, base_url="http://test"
        ) as client:
            with patch(
                "app.channels.whatsapp.handlers.settings"
            ) as mock_settings:
                mock_settings.whatsapp_verify_token = "test-token-123"
                resp = await client.get(
                    "/webhook/whatsapp",
                    params={
                        "hub.mode": "subscribe",
                        "hub.verify_token": "test-token-123",
                        "hub.challenge": "challenge_abc",
                    },
                )
        assert resp.status_code == 200
        assert resp.text == "challenge_abc"

    @pytest.mark.asyncio
    async def test_invalid_token_rejected(self):
        from httpx import ASGITransport, AsyncClient

        from app.api.main import app

        transport = ASGITransport(app=app)
        async with AsyncClient(
            transport=transport, base_url="http://test"
        ) as client:
            with patch(
                "app.channels.whatsapp.handlers.settings"
            ) as mock_settings:
                mock_settings.whatsapp_verify_token = "correct-token"
                resp = await client.get(
                    "/webhook/whatsapp",
                    params={
                        "hub.mode": "subscribe",
                        "hub.verify_token": "wrong-token",
                        "hub.challenge": "challenge_abc",
                    },
                )
        assert resp.status_code == 403

    @pytest.mark.asyncio
    async def test_missing_mode_rejected(self):
        from httpx import ASGITransport, AsyncClient

        from app.api.main import app

        transport = ASGITransport(app=app)
        async with AsyncClient(
            transport=transport, base_url="http://test"
        ) as client:
            with patch(
                "app.channels.whatsapp.handlers.settings"
            ) as mock_settings:
                mock_settings.whatsapp_verify_token = "test-token"
                resp = await client.get(
                    "/webhook/whatsapp",
                    params={
                        "hub.verify_token": "test-token",
                        "hub.challenge": "challenge_abc",
                    },
                )
        assert resp.status_code == 403


class TestHandleMessage:
    @pytest.mark.asyncio
    async def test_text_message_routes_to_agent(self):
        mock_user = MagicMock()
        mock_user.id = uuid.uuid4()

        with (
            patch(
                "app.channels.whatsapp.handlers.async_session_factory"
            ) as mock_sf,
            patch(
                "app.channels.whatsapp.handlers.route_message"
            ) as mock_route,
            patch(
                "app.channels.whatsapp.handlers._get_or_create_whatsapp_user"
            ) as mock_get_user,
        ):
            # Set up session mock
            mock_session = AsyncMock()
            mock_result = MagicMock()
            mock_channel = MagicMock()
            mock_result.scalar_one_or_none.return_value = mock_channel
            mock_session.execute = AsyncMock(return_value=mock_result)
            mock_session.commit = AsyncMock()
            mock_sf.return_value.__aenter__ = AsyncMock(
                return_value=mock_session
            )
            mock_sf.return_value.__aexit__ = AsyncMock(return_value=False)

            mock_get_user.return_value = (mock_user, False)
            mock_route.return_value = None

            message = {
                "from": "2348012345678",
                "type": "text",
                "text": {"body": "Hello Mika"},
            }
            await _handle_message(message)

            mock_route.assert_called_once()
            incoming = mock_route.call_args[0][0]
            assert incoming.text == "Hello Mika"
            assert incoming.channel_type == "whatsapp"
            assert incoming.channel_user_id == "2348012345678"

    @pytest.mark.asyncio
    async def test_non_text_message_ignored(self):
        with patch(
            "app.channels.whatsapp.handlers.async_session_factory"
        ):
            message = {
                "from": "2348012345678",
                "type": "image",
                "image": {"id": "123"},
            }
            # Should not raise
            await _handle_message(message)

    @pytest.mark.asyncio
    async def test_empty_text_ignored(self):
        with patch(
            "app.channels.whatsapp.handlers.async_session_factory"
        ):
            message = {
                "from": "2348012345678",
                "type": "text",
                "text": {"body": ""},
            }
            await _handle_message(message)


class TestGetOrCreateWhatsAppUser:
    @pytest.mark.asyncio
    async def test_existing_user_returned(self):
        mock_session = AsyncMock()
        existing_user = MagicMock()
        existing_user.id = uuid.uuid4()
        mock_channel = MagicMock()
        mock_channel.user_id = existing_user.id

        # First call: find channel
        channel_result = MagicMock()
        channel_result.scalar_one_or_none.return_value = mock_channel

        # Second call: find user
        user_result = MagicMock()
        user_result.scalar_one.return_value = existing_user

        mock_session.execute = AsyncMock(
            side_effect=[channel_result, user_result]
        )

        user, created = await _get_or_create_whatsapp_user(
            "2348012345678", mock_session
        )
        assert user == existing_user
        assert created is False

    @pytest.mark.asyncio
    async def test_new_user_created(self):
        mock_session = AsyncMock()

        # No existing channel
        channel_result = MagicMock()
        channel_result.scalar_one_or_none.return_value = None
        mock_session.execute = AsyncMock(return_value=channel_result)
        mock_session.add = MagicMock()
        mock_session.flush = AsyncMock()

        user, created = await _get_or_create_whatsapp_user(
            "2348099999999", mock_session
        )
        assert created is True
        assert user.preferred_channel == "whatsapp"
        # Should add user, channel, and consent
        assert mock_session.add.call_count == 3


class TestWhatsAppAdapter:
    @pytest.mark.asyncio
    async def test_send_message(self):
        from app.channels.base import OutgoingMessage
        from app.channels.whatsapp import WhatsAppAdapter

        adapter = WhatsAppAdapter()
        msg = OutgoingMessage(
            user_id="u1",
            channel_type="whatsapp",
            channel_user_id="2348012345678",
            text="Hello from Mika!",
        )

        with patch("app.channels.whatsapp.httpx.AsyncClient") as mock_client:
            mock_resp = MagicMock()
            mock_resp.status_code = 200
            mock_instance = AsyncMock()
            mock_instance.post = AsyncMock(return_value=mock_resp)
            mock_client.return_value.__aenter__ = AsyncMock(
                return_value=mock_instance
            )
            mock_client.return_value.__aexit__ = AsyncMock(
                return_value=False
            )

            await adapter.send_message(msg)

            mock_instance.post.assert_called_once()
            call_kwargs = mock_instance.post.call_args
            payload = call_kwargs.kwargs.get("json") or call_kwargs[1].get(
                "json"
            )
            assert payload["messaging_product"] == "whatsapp"
            assert payload["to"] == "2348012345678"
            assert payload["text"]["body"] == "Hello from Mika!"

    @pytest.mark.asyncio
    async def test_send_template(self):
        from app.channels.whatsapp import WhatsAppAdapter

        adapter = WhatsAppAdapter()

        with patch("app.channels.whatsapp.httpx.AsyncClient") as mock_client:
            mock_resp = MagicMock()
            mock_resp.status_code = 200
            mock_instance = AsyncMock()
            mock_instance.post = AsyncMock(return_value=mock_resp)
            mock_client.return_value.__aenter__ = AsyncMock(
                return_value=mock_instance
            )
            mock_client.return_value.__aexit__ = AsyncMock(
                return_value=False
            )

            await adapter.send_template(
                to="2348012345678",
                template_name="hello_world",
                parameters=["John"],
            )

            mock_instance.post.assert_called_once()
            call_kwargs = mock_instance.post.call_args
            payload = call_kwargs.kwargs.get("json") or call_kwargs[1].get(
                "json"
            )
            assert payload["type"] == "template"
            assert payload["template"]["name"] == "hello_world"
            assert len(payload["template"]["components"]) == 1

    @pytest.mark.asyncio
    async def test_typing_indicator_noop(self):
        from app.channels.whatsapp import WhatsAppAdapter

        adapter = WhatsAppAdapter()
        # Should not raise
        await adapter.send_typing_indicator("2348012345678")
