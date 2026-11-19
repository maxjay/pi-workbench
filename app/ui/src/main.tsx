import { render } from 'solid-js/web';
import '@fontsource-variable/archivo';
import '@fontsource-variable/jetbrains-mono';
import './theme.css';
import './app.css';
import './pull.css';
import { App } from './App';
import { connect } from './daemon';

void connect();
render(() => <App />, document.getElementById('root') as HTMLElement);
