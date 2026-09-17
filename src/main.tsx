import React from 'react';
import ReactDOM from 'react-dom/client';
import App from './App';
import StatisticsWindow from './components/StatisticsWindow';
import './App.css';

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    {window.location.hash === '#statistics' ? <StatisticsWindow /> : <App />}
  </React.StrictMode>,
);
