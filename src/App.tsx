import { AppShell } from "./components/shell/AppShell";
import { ArtistModeProvider } from "./lib/artistMode";
import { AuthorNameProvider } from "./lib/authorName";
import { ThemeProvider } from "./lib/theme";
import { RepositoryProvider } from "./lib/repository";

function App() {
  return (
    <RepositoryProvider>
      <ThemeProvider>
        <ArtistModeProvider>
          <AuthorNameProvider>
            <AppShell />
          </AuthorNameProvider>
        </ArtistModeProvider>
      </ThemeProvider>
    </RepositoryProvider>
  );
}

export default App;
