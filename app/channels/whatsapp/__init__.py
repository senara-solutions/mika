"""WhatsApp channel adapter using Meta Cloud API."""

import httpx

from app.channels.base import ChannelAdapter, OutgoingMessage
from app.channels.router import split_message
from app.common.logging import get_logger
from app.config import settings

logger = get_logger(__name__)

WHATSAPP_API_URL = "https://graph.facebook.com/v21.0"


class WhatsAppAdapter(ChannelAdapter):
    async def send_message(self, msg: OutgoingMessage) -> None:
        url = f"{WHATSAPP_API_URL}/{settings.whatsapp_phone_id}/messages"
        headers = {
            "Authorization": f"Bearer {settings.whatsapp_token}",
            "Content-Type": "application/json",
        }

        for chunk in split_message(msg.text, max_length=4096):
            payload = {
                "messaging_product": "whatsapp",
                "to": msg.channel_user_id,
                "type": "text",
                "text": {"body": chunk},
            }
            async with httpx.AsyncClient() as client:
                response = await client.post(
                    url, json=payload, headers=headers, timeout=30
                )
                if response.status_code != 200:
                    logger.warning(
                        "WhatsApp send failed",
                        extra={
                            "status": response.status_code,
                            "body": response.text,
                        },
                    )

    async def send_typing_indicator(
        self, channel_user_id: str
    ) -> None:
        # WhatsApp Cloud API doesn't have a direct typing indicator
        pass

    async def send_template(
        self,
        to: str,
        template_name: str,
        language_code: str = "en_US",
        parameters: list[str] | None = None,
    ) -> None:
        """Send an approved WhatsApp template message."""
        url = f"{WHATSAPP_API_URL}/{settings.whatsapp_phone_id}/messages"
        headers = {
            "Authorization": f"Bearer {settings.whatsapp_token}",
            "Content-Type": "application/json",
        }

        components = []
        if parameters:
            components.append({
                "type": "body",
                "parameters": [
                    {"type": "text", "text": p} for p in parameters
                ],
            })

        payload = {
            "messaging_product": "whatsapp",
            "to": to,
            "type": "template",
            "template": {
                "name": template_name,
                "language": {"code": language_code},
                "components": components,
            },
        }

        async with httpx.AsyncClient() as client:
            response = await client.post(
                url, json=payload, headers=headers, timeout=30
            )
            if response.status_code != 200:
                logger.warning(
                    "WhatsApp template send failed",
                    extra={"status": response.status_code},
                )
