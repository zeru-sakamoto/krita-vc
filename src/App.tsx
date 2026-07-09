import { AppShell } from "./components/shell/AppShell";
import { ArtistModeProvider } from "./lib/artistMode";
import { AuthorNameProvider } from "./lib/authorName";
import { ThemeProvider } from "./lib/theme";
import { RepositoryProvider } from "./lib/repository";
import { WindowChromeProvider } from "./lib/windowChrome";

function App() {
  return (
    <RepositoryProvider>
      <ThemeProvider>
        <ArtistModeProvider>
          <AuthorNameProvider>
            <WindowChromeProvider>
              <AppShell />
            </WindowChromeProvider>
          </AuthorNameProvider>
        </ArtistModeProvider>
      </ThemeProvider>
    </RepositoryProvider>
  );
}

export default App;
