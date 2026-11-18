// The window: one row on top, and the page of the tab you are on.

import { TopBar } from './components/TopBar';
import { Home } from './views/Home';

export function App() {
  return (
    <div class="window">
      <TopBar />
      <main class="content">
        <Home />
      </main>
    </div>
  );
}
