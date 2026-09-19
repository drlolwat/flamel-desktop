import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import './styles/globals.css';
import App from './App';

// Load Tauri adapter (self-guards: no-ops if not in Tauri or if Electron preload already ran)
if (!window.electronAPI) {
  import('./tauri');
}

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <App />
  </StrictMode>
);
