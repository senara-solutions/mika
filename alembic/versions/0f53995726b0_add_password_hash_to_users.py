"""add_password_hash_to_users

Revision ID: 0f53995726b0
Revises: dad5992ae0ca
Create Date: 2026-02-16 17:34:11.832336

"""
from typing import Sequence, Union

import sqlalchemy as sa
from alembic import op

# revision identifiers, used by Alembic.
revision: str = "0f53995726b0"
down_revision: Union[str, Sequence[str], None] = "dad5992ae0ca"
branch_labels: Union[str, Sequence[str], None] = None
depends_on: Union[str, Sequence[str], None] = None


def upgrade() -> None:
    op.add_column(
        "users",
        sa.Column("password_hash", sa.String(255), nullable=True),
    )


def downgrade() -> None:
    op.drop_column("users", "password_hash")
