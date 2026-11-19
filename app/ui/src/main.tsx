import { render } from 'solid-js/web';
import '@fontsource-variable/archivo';
import '@fontsource-variable/jetbrains-mono';
import './theme.css';
import './app.css';
import './pull.css';
import { App } from './App';
import { connect } from './daemon';
import { showTheme } from './state/theme';

// The theme picked in Settings is on the page before anything is drawn.
showTheme();
void connect();
render(() => <App />, document.getElementById('root') as HTMLElement);
