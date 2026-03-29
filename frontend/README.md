# StellarSwipe HUD Components

Minimal HUD system for tycoon stats with real-time updates and mobile optimization.

## Features

- **Composable stat widgets** for cash, income rate, and boosts
- **Debounced polling** to prevent excessive re-renders
- **Empty/zero state handling** with visual indicators
- **Mobile-responsive design** with touch-friendly interface
- **Performance optimized** with documented re-render budget

## Quick Start

```bash
cd frontend
npm install
npm run storybook
```

## Usage

```tsx
import { HUD } from './components/HUD';
import { StellarSwipeHUDAdapter } from './utils/stellarswipe-adapter';

const adapter = new StellarSwipeHUDAdapter('CONTRACT_ADDRESS');

function App() {
  return (
    <HUD 
      onStatsUpdate={() => adapter.fetchTycoonStats()}
      pollInterval={5000}
      initialStats={{ cash: 0, incomeRate: 0, boosts: 0 }}
    />
  );
}
```

## QA Checklist

### Desktop Testing
- [ ] HUD renders in top-right corner
- [ ] Stats update every 5 seconds
- [ ] Loading state shows opacity change
- [ ] Empty states display "(empty)" suffix
- [ ] Values format correctly with commas

### Mobile Testing (< 768px)
- [ ] HUD repositions to bottom of screen
- [ ] Components stack horizontally
- [ ] Touch targets are minimum 44px
- [ ] Text remains readable at smaller sizes
- [ ] Backdrop blur works on mobile browsers

### Performance Testing
- [ ] Initial render < 16ms
- [ ] Update renders < 8ms
- [ ] No memory leaks after 100+ updates
- [ ] Polling stops when component unmounts
- [ ] Network requests are properly debounced

## Storybook States

Access visual documentation at `http://localhost:6006`:

- **Default**: Normal operation with mock data
- **Empty State**: All values at zero
- **High Values**: Large numbers with proper formatting
- **Loading**: Demonstrates loading state transitions
- **Mobile**: Mobile viewport testing

## Integration

The HUD integrates with StellarSwipe contracts via the `StellarSwipeHUDAdapter` utility. Replace mock implementation with actual Soroban contract calls for production use.