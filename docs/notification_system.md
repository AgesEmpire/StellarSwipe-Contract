# Real-Time Notification System

Stellar Swipe supports a WebSocket-powered notification service for real-time user alerts.

## Features

- Alerts for trades, rewards, fee updates, and important protocol events.
- Subscription management through a lightweight WebSocket API.
- Client-side connection recovery and retry logic.
- Notifications are delivered as discrete messages with type, title, body, and timestamp.

## Integration

Frontend clients should use the `useRealtimeNotifications` hook to receive live alerts and reconnect automatically.

Backend services can broadcast events over a WebSocket channel and persist user subscriptions for delivery guarantees.
