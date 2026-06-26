import { AppShell } from "./components/shell/AppShell";
import { ArtistModeProvider } from "./lib/artistMode";
import { RepositoryProvider } from "./lib/repository";

function App() {
  return (
    <RepositoryProvider>
      <ArtistModeProvider>
        <AppShell />
      </ArtistModeProvider>
    </RepositoryProvider>
  );
}

export default App;
