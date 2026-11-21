import React from "react";
import { NotificationMessage } from "../hooks/useRealtimeNotifications";

interface NotificationBannerProps {
  notifications: NotificationMessage[];
  connected: boolean;
  error?: string;
}

export const NotificationBanner: React.FC<NotificationBannerProps> = ({
  notifications,
  connected,
  error,
}) => {
  return (
    <div className={`notification-banner ${connected ? "connected" : "disconnected"}`}>
      <div className="notification-banner-header">
        <span>{connected ? "Live notifications enabled" : "Disconnected"}</span>
        {error ? <span className="notification-banner-error">{error}</span> : null}
      </div>
      <div className="notification-banner-list">
        {notifications.slice(0, 3).map((notification) => (
          <div key={notification.id} className="notification-banner-item">
            <strong>{notification.title}</strong>
            <p>{notification.body}</p>
          </div>
        ))}
      </div>
    </div>
  );
};

export default NotificationBanner;
