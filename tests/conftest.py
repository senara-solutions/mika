"""Shared test fixtures."""

import uuid
from unittest.mock import AsyncMock, MagicMock

import pytest
from langchain_core.messages import AIMessage

# Import all models so SQLAlchemy relationships resolve correctly
import app.models.audit  # noqa: F401
import app.models.channel  # noqa: F401
import app.models.consent  # noqa: F401
import app.models.conversation  # noqa: F401
import app.models.user  # noqa: F401


@pytest.fixture
def user_id() -> str:
    return str(uuid.uuid4())


@pytest.fixture
def mock_llm() -> MagicMock:
    """Mock ChatAnthropic that returns a simple response."""
    llm = MagicMock()
    llm.ainvoke = AsyncMock(return_value=AIMessage(content="Mock response"))
    llm.bind_tools = MagicMock(return_value=llm)
    return llm


@pytest.fixture
def mock_neo4j_session() -> AsyncMock:
    """Mock Neo4j async session."""
    session = AsyncMock()
    session.run = AsyncMock(return_value=MagicMock(data=MagicMock(return_value=[])))
    return session


@pytest.fixture
def mock_neo4j_driver(mock_neo4j_session: AsyncMock) -> MagicMock:
    """Mock Neo4j async driver."""
    driver = MagicMock()
    driver.session = MagicMock(return_value=mock_neo4j_session)
    mock_neo4j_session.__aenter__ = AsyncMock(return_value=mock_neo4j_session)
    mock_neo4j_session.__aexit__ = AsyncMock(return_value=None)
    return driver
