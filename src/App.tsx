import { AppShell } from "./components/shell/AppShell";
import { ArtistModeProvider } from "./lib/artistMode";
import { LegacyHistoryProvider } from "./lib/legacyHistory";
import { AuthorNameProvider } from "./lib/authorName";
import { ThemeProvider } from "./lib/theme";
import { RepositoryProvider } from "./lib/repository";
import { WindowChromeProvider } from "./lib/windowChrome";
import { CpuBudgetProvider } from "./lib/cpuBudget";
import { ToastProvider } from "./lib/toast";
import { TourProvider } from "./lib/tour";

function App() {
  return (
    <ToastProvider>
      <RepositoryProvider>
        <ThemeProvider>
          <ArtistModeProvider>
            <LegacyHistoryProvider>
              <AuthorNameProvider>
                <WindowChromeProvider>
                  <CpuBudgetProvider>
                    <TourProvider>
                      <AppShell />
                    </TourProvider>
                  </CpuBudgetProvider>
                </WindowChromeProvider>
              </AuthorNameProvider>
            </LegacyHistoryProvider>
          </ArtistModeProvider>
        </ThemeProvider>
      </RepositoryProvider>
    </ToastProvider>
  );
}

export default App;
