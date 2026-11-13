/** @type {import('tailwindcss').Config} */
export default {
  content: ['./index.html', './src/**/*.{ts,tsx}'],
  theme: {
    extend: {
      colors: {
        bg: '#050505',
        panel: '#101012',
        line: '#1f1f23',
        mute: '#8b8b93',
        accent: '#00E599',
      },
      fontFamily: {
        sans: ['Inter', 'system-ui', 'Roboto', 'sans-serif'],
      },
    },
  },
  plugins: [],
};
