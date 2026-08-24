export interface NotificationSubscription {
  eventTypes?: string[];
  userAddress?: string;
  locale?: string;
}

export interface NotificationPayload {
  type: string;
  title: string;
  message: string;
  metadata?: Record<string, unknown>;
  timestamp: number;
}

export const buildNotificationUrl = (baseUrl: string, params?: Record<string, string>) => {
  const url = new URL(baseUrl);
  if (params) {
    Object.entries(params).forEach(([key, value]) => url.searchParams.set(key, value));
  }
  return url.toString();
};

export const createSubscriptionPayload = (subscription: NotificationSubscription) => ({
  type: "subscribe",
  payload: subscription,
});

export const parseNotificationMessage = (raw: string): NotificationPayload | null => {
  try {
    const parsed = JSON.parse(raw);
    if (typeof parsed?.type !== "string" || typeof parsed?.title !== "string") {
      return null;
    }
    return parsed as NotificationPayload;
  } catch {
    return null;
  }
};
