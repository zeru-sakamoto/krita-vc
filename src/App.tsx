import { AppShell } from "./components/shell/AppShell";
import { ArtistModeProvider } from "./lib/artistMode";
import { AuthorNameProvider } from "./lib/authorName";
import { RepositoryProvider } from "./lib/repository";

function App() {
  return (
    <RepositoryProvider>
      <ArtistModeProvider>
        <AuthorNameProvider>
          <AppShell />
        </AuthorNameProvider>
      </ArtistModeProvider>
    </RepositoryProvider>
  );
}

export default App;
