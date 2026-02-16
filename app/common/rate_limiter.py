"""Redis-based rate limiter for proactive messages."""

import redis.asyncio as redis

from app.config import settings

MAX_PER_HOUR = 1
MAX_PER_DAY = 3


def _get_redis() -> redis.Redis:
    """Get async Redis client."""
    return redis.from_url(settings.redis_url, decode_responses=True)


async def can_send_proactive(user_id: str) -> bool:
    """Check if we can send a proactive message to this user.

    Enforces:
    - Max 1 proactive message per user per hour
    - Max 3 proactive messages per user per day
    """
    r = _get_redis()
    try:
        hourly_key = f"proactive:hourly:{user_id}"
        daily_key = f"proactive:daily:{user_id}"

        hourly_count = await r.get(hourly_key)
        daily_count = await r.get(daily_key)

        if hourly_count and int(hourly_count) >= MAX_PER_HOUR:
            return False
        if daily_count and int(daily_count) >= MAX_PER_DAY:
            return False

        return True
    finally:
        await r.aclose()


async def record_proactive_send(user_id: str) -> None:
    """Record that a proactive message was sent."""
    r = _get_redis()
    try:
        hourly_key = f"proactive:hourly:{user_id}"
        daily_key = f"proactive:daily:{user_id}"

        pipe = r.pipeline()
        pipe.incr(hourly_key)
        pipe.expire(hourly_key, 3600)  # 1 hour TTL
        pipe.incr(daily_key)
        pipe.expire(daily_key, 86400)  # 24 hour TTL
        await pipe.execute()
    finally:
        await r.aclose()
