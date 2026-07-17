import { AppShell } from "./components/shell/AppShell";
import { ArtistModeProvider } from "./lib/artistMode";
import { AuthorNameProvider } from "./lib/authorName";
import { ThemeProvider } from "./lib/theme";
import { RepositoryProvider } from "./lib/repository";
import { WindowChromeProvider } from "./lib/windowChrome";
import { ToastProvider } from "./lib/toast";

function App() {
  return (
    <RepositoryProvider>
      <ToastProvider>
        <ThemeProvider>
          <ArtistModeProvider>
            <AuthorNameProvider>
              <WindowChromeProvider>
                <AppShell />
              </WindowChromeProvider>
            </AuthorNameProvider>
          </ArtistModeProvider>
        </ThemeProvider>
      </ToastProvider>
    </RepositoryProvider>
  );
}

export default App;
