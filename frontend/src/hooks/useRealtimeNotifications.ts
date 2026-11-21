import { useEffect, useState, useRef } from "react";

export interface NotificationMessage {
  id: string;
  category: "trade" | "reward" | "protocol" | "system";
  title: string;
  body: string;
  timestamp: number;
}

export interface NotificationState {
  connected: boolean;
  messages: NotificationMessage[];
  lastError?: string;
}

export const useRealtimeNotifications = (url: string, enabled = true) => {
  const [messages, setMessages] = useState<NotificationMessage[]>([]);
  const [connected, setConnected] = useState(false);
  const [lastError, setLastError] = useState<string | undefined>(undefined);
  const socketRef = useRef<WebSocket | null>(null);

  useEffect(() => {
    if (!enabled || !url) {
      return;
    }

    const ws = new WebSocket(url);
    socketRef.current = ws;

    ws.addEventListener("open", () => {
      setConnected(true);
      setLastError(undefined);
    });

    ws.addEventListener("message", (event) => {
      try {
        const data = JSON.parse(event.data) as NotificationMessage;
        setMessages((current) => [data, ...current].slice(0, 50));
      } catch (error) {
        setLastError("Invalid notification payload received.");
      }
    });

    ws.addEventListener("close", () => {
      setConnected(false);
      setTimeout(() => {
        if (socketRef.current === ws) {
          setLastError("Connection closed. Reconnecting...");
          socketRef.current = null;
        }
      }, 2000);
    });

    ws.addEventListener("error", () => {
      setLastError("WebSocket connection failed.");
      setConnected(false);
    });

    return () => {
      ws.close();
      socketRef.current = null;
    };
  }, [url, enabled]);

  return {
    connected,
    messages,
    lastError,
    subscribe: (subscriptionPayload: Record<string, unknown>) => {
      if (socketRef.current?.readyState === WebSocket.OPEN) {
        socketRef.current.send(JSON.stringify(subscriptionPayload));
      }
    },
  } as NotificationState & {
    subscribe: (payload: Record<string, unknown>) => void;
  };
};
