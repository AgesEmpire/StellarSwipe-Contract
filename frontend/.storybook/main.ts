import type { StorybookConfig } from '@storybook/react';

const config: StorybookConfig = {
  stories: ['../stories/**/*.stories.@(js|jsx|ts|tsx)'],
  addons: [
    '@storybook/addon-essentials',
  ],
  framework: {
    name: '@storybook/react',
    options: {},
  },
  typescript: {
    reactDocgen: 'react-docgen-typescript',
  },
};

export default config;