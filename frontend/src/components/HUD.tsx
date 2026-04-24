import React, { useState, useCallback } from 'react';
import React, { useState, useEffect, useCallback } from 'react';
import { useDebouncedPolling } from '../hooks/useDebouncedPolling';

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

  const fetchStats = useCallback(async () => {
    if (!onStatsUpdate) return;
    
    setIsLoading(true);
    try {
      const newStats = await onStatsUpdate();
      setStats(newStats);
    } catch (error) {
      console.error('Failed to fetch stats:', error);
    } finally {
      setIsLoading(false);
    }
  }, [onStatsUpdate]);

  useDebouncedPolling(fetchStats, pollInterval);

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
