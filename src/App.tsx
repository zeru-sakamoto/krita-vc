import { AppShell } from "./components/shell/AppShell";
import { ArtistModeProvider } from "./lib/artistMode";
import { LegacyHistoryProvider } from "./lib/legacyHistory";
import { AuthorNameProvider } from "./lib/authorName";
import { ThemeProvider } from "./lib/theme";
import { RepositoryProvider } from "./lib/repository";
import { WindowChromeProvider } from "./lib/windowChrome";
import { CpuBudgetProvider } from "./lib/cpuBudget";
import { ToastProvider } from "./lib/toast";
import { RightClickGuard } from "./lib/rightClickGuard";
import { TourProvider } from "./lib/tour";
import { OnboardingProvider } from "./lib/onboarding";

function App() {
  return (
    <ToastProvider>
      <RightClickGuard />
      <RepositoryProvider>
        <ThemeProvider>
          <ArtistModeProvider>
            <LegacyHistoryProvider>
              <AuthorNameProvider>
                <WindowChromeProvider>
                  <CpuBudgetProvider>
                    <OnboardingProvider>
                      <TourProvider>
                        <AppShell />
                      </TourProvider>
                    </OnboardingProvider>
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
