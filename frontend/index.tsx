import 'normalize.css';
import './index.css'

import * as preact from 'preact';
import { App } from './src/app';
preact.render(<App />, document.getElementById('app')!);
