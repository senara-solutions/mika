"""add_google_credentials_to_users

Revision ID: a3b7c91d4e5f
Revises: 0f53995726b0
Create Date: 2026-02-16 20:00:00.000000

"""
from typing import Sequence, Union

import sqlalchemy as sa
from alembic import op
from sqlalchemy.dialects.postgresql import JSONB

# revision identifiers, used by Alembic.
revision: str = "a3b7c91d4e5f"
down_revision: Union[str, Sequence[str], None] = "0f53995726b0"
branch_labels: Union[str, Sequence[str], None] = None
depends_on: Union[str, Sequence[str], None] = None


def upgrade() -> None:
    op.add_column(
        "users",
        sa.Column("google_credentials", JSONB, nullable=True),
    )


def downgrade() -> None:
    op.drop_column("users", "google_credentials")
