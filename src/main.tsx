import React from 'react';
import { createRoot } from 'react-dom/client';
import App from './App';
import { applyInitialTheme } from './hooks/useTheme';
import './styles/tokens.css';
import './styles/base.css';
import './styles/components.css';
import './styles/layout.css';
import './styles/views.css';

applyInitialTheme();

createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
