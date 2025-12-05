import 'normalize.css';
import './index.css'

import * as preact from 'preact';
import { App } from './app';
preact.render(<App />, document.getElementById('app')!);
