import React, { useState, useCallback } from 'react';
import React, { useState, useEffect, useCallback } from 'react';
import { useDebouncedPolling } from '../hooks/useDebouncedPolling';
import { FetchError } from '../utils/stellarswipe-adapter';

interface TycoonStats {
  cash: number;
  incomeRate: number;
  boosts: number;
}

interface StatWidgetProps {
  label: string;
  value: number;
  format?: (value: number) => string;
  className?: string;
}

const StatWidget: React.FC<StatWidgetProps> = ({
  label,
  value,
  format = (v) => v.toLocaleString(),
  className = '',
const StatWidget: React.FC<StatWidgetProps> = ({ 
  label, 
  value, 
  format = (v) => v.toLocaleString(),
  className = ''
}) => (
  <div className={`stat-widget ${className}`}>
    <div className="stat-label">{label}</div>
    <div className="stat-value">{format(value)}</div>
  </div>
);

interface HUDProps {
  onStatsUpdate?: () => Promise<TycoonStats>;
  pollInterval?: number;
  initialStats?: TycoonStats;
}

export const HUD: React.FC<HUDProps> = ({
  onStatsUpdate,
  pollInterval = 5000,
  initialStats = { cash: 0, incomeRate: 0, boosts: 0 },
export const HUD: React.FC<HUDProps> = ({ 
  onStatsUpdate,
  pollInterval = 5000,
  initialStats = { cash: 0, incomeRate: 0, boosts: 0 }
}) => {
  const [stats, setStats] = useState<TycoonStats>(initialStats);
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<{ message: string; kind: 'network' | 'server' | 'unknown' } | null>(null);

  const fetchStats = useCallback(async () => {
    if (!onStatsUpdate) return;
    
    setIsLoading(true);
    setError(null);
    try {
      const newStats = await onStatsUpdate();
      setStats(newStats);
    } catch (err) {
      if (err instanceof FetchError) {
        setError({ message: err.message, kind: err.kind });
      } else {
        setError({ message: 'An unexpected error occurred.', kind: 'unknown' });
      }
    } finally {
      setIsLoading(false);
    }
  }, [onStatsUpdate]);

  useDebouncedPolling(fetchStats, pollInterval, !error);

  if (error) {
    return (
      <div className="hud hud-error" role="alert">
        <span className={`hud-error-badge hud-error-badge--${error.kind}`}>
          {error.kind === 'network' ? 'Network Error' : 'Server Error'}
        </span>
        <span className="hud-error-message">{error.message}</span>
        <button className="hud-retry-btn" onClick={fetchStats} disabled={isLoading}>
          {isLoading ? 'Retrying…' : 'Retry'}
        </button>
      </div>
    );
  }

  return (
    <div className={`hud ${isLoading ? 'loading' : ''}`}>
      <StatWidget label="Cash" value={stats.cash} format={(v) => `$${v.toLocaleString()}`} className="cash-widget" />
      <StatWidget label="Income Rate" value={stats.incomeRate} format={(v) => `$${v.toLocaleString()}/min`} className="income-widget" />
      <StatWidget label="Boosts" value={stats.boosts} format={(v) => `${v}x`} className="boost-widget" />
  const formatCash = (value: number) => `$${value.toLocaleString()}`;
  const formatRate = (value: number) => `$${value.toLocaleString()}/min`;
  const formatBoosts = (value: number) => `${value}x`;

  return (
    <div className={`hud ${isLoading ? 'loading' : ''}`}>
      <StatWidget 
        label="Cash" 
        value={stats.cash} 
        format={formatCash}
        className="cash-widget"
      />
      <StatWidget 
        label="Income Rate" 
        value={stats.incomeRate} 
        format={formatRate}
        className="income-widget"
      />
      <StatWidget 
        label="Boosts" 
        value={stats.boosts} 
        format={formatBoosts}
        className="boost-widget"
      />
    </div>
  );
};

export default HUD;
export default HUD;
