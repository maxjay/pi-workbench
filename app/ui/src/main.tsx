import { render } from 'solid-js/web';
import { App } from './App';
import { connect } from './agent';
import './theme.css';
import './app.css';

void connect();
render(() => <App />, document.getElementById('root') as HTMLElement);
