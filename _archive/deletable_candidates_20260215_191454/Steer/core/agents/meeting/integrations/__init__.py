"""Integrations namespace for meeting agent."""

from .sync import IntegrationConfig, load_provider_config, sync_action_items

__all__ = [
    "IntegrationConfig",
    "load_provider_config",
    "sync_action_items",
]

