import React from 'react';
import ReactDOM from 'react-dom/client';
import App from './app/App';
import '@/i18n';
import './App.css';

ReactDOM.createRoot(document.getElementById('root') as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>
);
