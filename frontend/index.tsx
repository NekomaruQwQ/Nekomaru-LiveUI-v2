import 'normalize.css';
import './index.css';

import { createRoot } from 'react-dom/client';

import { App } from './src/app';

createRoot(document.getElementById('app')!).render(<App />);
