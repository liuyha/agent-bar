import type { Config } from 'tailwindcss';
import animate from 'tailwindcss-animate';

export default {
  content: ['./index.html', './src/**/*.{ts,tsx}'],
  // Keep the existing native-panel reset and typography during incremental migration.
  corePlugins: { preflight: false },
  theme: {
    extend: {
      colors: {
        border: 'var(--line)',
        input: 'var(--line)',
        ring: 'var(--focus)',
        background: 'var(--app-bg)',
        foreground: 'var(--text)',
        primary: { DEFAULT: 'var(--primary-bg)', foreground: 'var(--primary-text)' },
        secondary: { DEFAULT: 'var(--subtle-bg)', foreground: 'var(--secondary)' },
        muted: { DEFAULT: 'var(--subtle-bg)', foreground: 'var(--muted)' },
        accent: { DEFAULT: 'var(--hover-bg)', foreground: 'var(--text)' },
        destructive: { DEFAULT: 'var(--danger)', foreground: 'var(--primary-text)' },
        card: { DEFAULT: 'var(--card-bg)', foreground: 'var(--text)' },
        popover: { DEFAULT: 'var(--card-bg)', foreground: 'var(--text)' },
      },
      borderRadius: { lg: '11px', md: '7px', sm: '4px' },
    },
  },
  plugins: [animate],
} satisfies Config;
