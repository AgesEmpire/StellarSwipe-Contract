# HUD Performance Budget

## Re-render Performance Requirements

### Target Metrics
- **Initial render**: < 16ms (60fps)
- **Update renders**: < 8ms (120fps capable)
- **Memory usage**: < 5MB for HUD components
- **Polling overhead**: < 1ms per poll cycle

### Optimization Strategies

#### 1. Debounced Polling
- Default 5s interval prevents excessive API calls
- Batches multiple rapid updates into single render
- Cancels pending requests on unmount

#### 2. Memoization
- StatWidget components are pure (no side effects)
- Value formatting functions are stable references
- Stats object updates only trigger re-render when values change

#### 3. Minimal DOM Updates
- CSS transitions handle loading states
- Fixed positioning prevents layout thrashing
- Composable widgets allow selective updates

### Performance Monitoring

```typescript
// Add to HUD component for monitoring
const renderCount = useRef(0);
useEffect(() => {
  renderCount.current++;
  if (renderCount.current > 100) {
    console.warn('HUD: High render count detected');
  }
});
```

### Mobile Considerations
- Reduced poll frequency on mobile (10s default)
- Simplified animations for lower-end devices
- Touch-friendly sizing (44px minimum touch targets)