"""Tests for privacy export and deletion APIs."""

import json
import uuid
import zipfile
from io import BytesIO
from unittest.mock import AsyncMock, MagicMock, patch

import pytest

from app.api.routes.privacy import (
    _gather_memory_data,
    _gather_postgres_data,
)


class TestGatherPostgresData:
    @pytest.mark.asyncio
    async def test_gather_returns_profile_channels_conversations(self):
        user_id = str(uuid.uuid4())
        mock_user = MagicMock()
        mock_user.id = user_id
        mock_user.email = "test@example.com"
        mock_user.timezone = "UTC"
        mock_user.preferred_channel = "telegram"
        mock_user.onboarding_completed = True
        mock_user.created_at = "2026-01-01"
        mock_user.last_active_at = None

        mock_channel = MagicMock()
        mock_channel.channel_type = "telegram"
        mock_channel.channel_user_id = "12345"
        mock_channel.connected_at = "2026-01-01"

        mock_conv = MagicMock()
        mock_conv.role = "user"
        mock_conv.content = "Hello"
        mock_conv.channel_type = "telegram"
        mock_conv.created_at = "2026-01-01"

        mock_session = AsyncMock()

        # Three queries: user, channels, conversations
        user_result = MagicMock()
        user_result.scalar_one.return_value = mock_user

        ch_result = MagicMock()
        ch_scalars = MagicMock()
        ch_scalars.all.return_value = [mock_channel]
        ch_result.scalars.return_value = ch_scalars

        conv_result = MagicMock()
        conv_scalars = MagicMock()
        conv_scalars.all.return_value = [mock_conv]
        conv_result.scalars.return_value = conv_scalars

        mock_session.execute = AsyncMock(
            side_effect=[user_result, ch_result, conv_result]
        )

        with patch(
            "app.api.routes.privacy.async_session_factory"
        ) as mock_sf:
            mock_sf.return_value.__aenter__ = AsyncMock(
                return_value=mock_session
            )
            mock_sf.return_value.__aexit__ = AsyncMock(return_value=False)

            profile, channels, conversations = await _gather_postgres_data(
                user_id
            )

        assert profile["email"] == "test@example.com"
        assert len(channels) == 1
        assert channels[0]["channel_type"] == "telegram"
        assert len(conversations) == 1
        assert conversations[0]["content"] == "Hello"


class TestGatherMemoryData:
    @pytest.mark.asyncio
    async def test_returns_nodes_on_success(self):
        mock_result = AsyncMock()
        mock_result.data = AsyncMock(
            return_value=[
                {
                    "labels": ["Person"],
                    "props": {
                        "canonical_name": "Sarah",
                        "user_id": "u1",
                        "description_hash": "abc",
                    },
                }
            ]
        )

        mock_session = AsyncMock()
        mock_session.run = AsyncMock(return_value=mock_result)

        mock_driver = MagicMock()
        mock_driver.session.return_value.__aenter__ = AsyncMock(
            return_value=mock_session
        )
        mock_driver.session.return_value.__aexit__ = AsyncMock(
            return_value=False
        )

        with patch(
            "app.memory.driver.get_neo4j_driver",
            return_value=mock_driver,
        ):
            nodes = await _gather_memory_data("u1")

        assert len(nodes) == 1
        assert nodes[0]["type"] == "Person"
        assert "description_hash" not in nodes[0]["properties"]

    @pytest.mark.asyncio
    async def test_returns_empty_on_error(self):
        with patch(
            "app.memory.driver.get_neo4j_driver",
            side_effect=Exception("No Neo4j"),
        ):
            nodes = await _gather_memory_data("u1")
        assert nodes == []


class TestDeleteNeo4jData:
    @pytest.mark.asyncio
    async def test_deletes_memory_and_user_nodes(self):
        from app.api.routes.privacy import _delete_neo4j_data

        mock_session = AsyncMock()
        mock_session.run = AsyncMock()

        mock_driver = MagicMock()
        mock_driver.session.return_value.__aenter__ = AsyncMock(
            return_value=mock_session
        )
        mock_driver.session.return_value.__aexit__ = AsyncMock(
            return_value=False
        )

        with patch(
            "app.memory.driver.get_neo4j_driver",
            return_value=mock_driver,
        ):
            await _delete_neo4j_data("u1")

        # Two calls: delete memory nodes, then delete user node
        assert mock_session.run.call_count == 2


class TestDeletePostgresUser:
    @pytest.mark.asyncio
    async def test_deletes_existing_user(self):
        from app.api.routes.privacy import _delete_postgres_user

        mock_user = MagicMock()
        mock_session = AsyncMock()
        result = MagicMock()
        result.scalar_one_or_none.return_value = mock_user
        mock_session.execute = AsyncMock(return_value=result)
        mock_session.delete = AsyncMock()
        mock_session.commit = AsyncMock()

        with patch(
            "app.api.routes.privacy.async_session_factory"
        ) as mock_sf:
            mock_sf.return_value.__aenter__ = AsyncMock(
                return_value=mock_session
            )
            mock_sf.return_value.__aexit__ = AsyncMock(return_value=False)

            await _delete_postgres_user("u1")

        mock_session.delete.assert_called_once_with(mock_user)
        mock_session.commit.assert_called_once()

    @pytest.mark.asyncio
    async def test_noop_for_missing_user(self):
        from app.api.routes.privacy import _delete_postgres_user

        mock_session = AsyncMock()
        result = MagicMock()
        result.scalar_one_or_none.return_value = None
        mock_session.execute = AsyncMock(return_value=result)

        with patch(
            "app.api.routes.privacy.async_session_factory"
        ) as mock_sf:
            mock_sf.return_value.__aenter__ = AsyncMock(
                return_value=mock_session
            )
            mock_sf.return_value.__aexit__ = AsyncMock(return_value=False)

            await _delete_postgres_user("u1")

        mock_session.delete.assert_not_called()


class TestExportEndpoint:
    @pytest.mark.asyncio
    async def test_export_returns_zip(self):
        from httpx import ASGITransport, AsyncClient

        from app.api.main import app

        mock_user = MagicMock()
        mock_user.id = uuid.uuid4()

        with (
            patch(
                "app.api.middleware.validate_session_token",
                return_value=str(mock_user.id),
            ),
            patch(
                "app.api.middleware.async_session_factory"
            ) as mock_sf,
            patch(
                "app.api.routes.privacy._gather_postgres_data"
            ) as mock_pg,
            patch(
                "app.api.routes.privacy._gather_memory_data"
            ) as mock_mem,
        ):
            # Mock middleware user lookup
            mock_session = AsyncMock()
            result = MagicMock()
            result.scalar_one_or_none.return_value = mock_user
            mock_session.execute = AsyncMock(return_value=result)
            mock_sf.return_value.__aenter__ = AsyncMock(
                return_value=mock_session
            )
            mock_sf.return_value.__aexit__ = AsyncMock(return_value=False)

            mock_pg.return_value = (
                {"email": "test@example.com"},
                [],
                [],
            )
            mock_mem.return_value = []

            transport = ASGITransport(app=app)
            async with AsyncClient(
                transport=transport, base_url="http://test"
            ) as client:
                resp = await client.post(
                    "/api/privacy/export",
                    cookies={"mika_session": "valid-token"},
                )

        assert resp.status_code == 200
        assert "application/zip" in resp.headers["content-type"]

        # Verify ZIP contents
        zf = zipfile.ZipFile(BytesIO(resp.content))
        assert "profile.json" in zf.namelist()
        assert "conversations.json" in zf.namelist()
        assert "memory.json" in zf.namelist()

        profile = json.loads(zf.read("profile.json"))
        assert profile["email"] == "test@example.com"


class TestDeleteEndpoint:
    @pytest.mark.asyncio
    async def test_delete_returns_ok(self):
        from httpx import ASGITransport, AsyncClient

        from app.api.main import app

        mock_user = MagicMock()
        mock_user.id = uuid.uuid4()

        with (
            patch(
                "app.api.middleware.validate_session_token",
                return_value=str(mock_user.id),
            ),
            patch(
                "app.api.middleware.async_session_factory"
            ) as mock_sf,
            patch(
                "app.api.routes.privacy._delete_neo4j_data"
            ) as mock_neo4j,
            patch(
                "app.api.routes.privacy._delete_postgres_user"
            ) as mock_pg,
            patch(
                "app.api.routes.privacy._clear_redis_cache"
            ) as mock_redis,
        ):
            mock_session = AsyncMock()
            result = MagicMock()
            result.scalar_one_or_none.return_value = mock_user
            mock_session.execute = AsyncMock(return_value=result)
            mock_sf.return_value.__aenter__ = AsyncMock(
                return_value=mock_session
            )
            mock_sf.return_value.__aexit__ = AsyncMock(return_value=False)

            transport = ASGITransport(app=app)
            async with AsyncClient(
                transport=transport, base_url="http://test"
            ) as client:
                resp = await client.post(
                    "/api/privacy/delete",
                    cookies={"mika_session": "valid-token"},
                )

        assert resp.status_code == 200
        data = resp.json()
        assert data["ok"] is True
        mock_neo4j.assert_called_once()
        mock_pg.assert_called_once()
        mock_redis.assert_called_once()

    @pytest.mark.asyncio
    async def test_delete_unauthorized(self):
        from httpx import ASGITransport, AsyncClient

        from app.api.main import app

        transport = ASGITransport(app=app)
        async with AsyncClient(
            transport=transport, base_url="http://test"
        ) as client:
            resp = await client.post("/api/privacy/delete")

        assert resp.status_code == 401
