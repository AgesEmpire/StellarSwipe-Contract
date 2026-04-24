import type { Meta, StoryObj } from '@storybook/react';
import { HUD } from '../src/components/HUD';
import '../src/components/HUD.css';

const meta: Meta<typeof HUD> = {
  title: 'HUD/TycoonStats',
  component: HUD,
  parameters: { layout: 'fullscreen' },
  argTypes: { pollInterval: { control: 'number' } },
  parameters: {
    layout: 'fullscreen',
  },
  argTypes: {
    pollInterval: { control: 'number' },
  },
};

export default meta;
type Story = StoryObj<typeof meta>;

const mockStatsUpdate = async () => {
  await new Promise((resolve) => setTimeout(resolve, 500));
// Mock stats update function
const mockStatsUpdate = async () => {
  await new Promise(resolve => setTimeout(resolve, 500));
  return {
    cash: Math.floor(Math.random() * 1000000),
    incomeRate: Math.floor(Math.random() * 5000),
    boosts: Math.floor(Math.random() * 10) + 1,
  };
};

export const Default: Story = {
  args: { initialStats: { cash: 50000, incomeRate: 1200, boosts: 3 }, onStatsUpdate: mockStatsUpdate, pollInterval: 3000 },
};

export const EmptyState: Story = {
  args: { initialStats: { cash: 0, incomeRate: 0, boosts: 0 } },
};

export const HighValues: Story = {
  args: { initialStats: { cash: 999999999, incomeRate: 50000, boosts: 25 } },
  args: {
    initialStats: { cash: 50000, incomeRate: 1200, boosts: 3 },
    onStatsUpdate: mockStatsUpdate,
    pollInterval: 3000,
  },
};

export const EmptyState: Story = {
  args: {
    initialStats: { cash: 0, incomeRate: 0, boosts: 0 },
  },
};

export const HighValues: Story = {
  args: {
    initialStats: { cash: 999999999, incomeRate: 50000, boosts: 25 },
  },
};

export const Loading: Story = {
  args: {
    initialStats: { cash: 25000, incomeRate: 800, boosts: 2 },
    onStatsUpdate: async () => {
      await new Promise((resolve) => setTimeout(resolve, 2000));
      await new Promise(resolve => setTimeout(resolve, 2000));
      return { cash: 26000, incomeRate: 850, boosts: 2 };
    },
    pollInterval: 1000,
  },
};

export const Mobile: Story = {
  args: { initialStats: { cash: 75000, incomeRate: 2500, boosts: 5 } },
  parameters: { viewport: { defaultViewport: 'mobile1' } },
};
  args: {
    initialStats: { cash: 75000, incomeRate: 2500, boosts: 5 },
  },
  parameters: {
    viewport: {
      defaultViewport: 'mobile1',
    },
  },
};
