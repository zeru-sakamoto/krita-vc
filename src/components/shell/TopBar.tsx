import { CaretDown, FolderOpen, Plus } from "@phosphor-icons/react";
import { Menu, type MenuItem } from "../ui/Menu";
import { useRepository } from "../../lib/repository";

/**
 * Slim top bar spanning the window. Hosts the repository switcher (a folder the
 * user has designated). Local-only VCS — no remote/fetch/push affordances.
 * (DESIGN.md → Layout & App Shell → Top bar)
 */
export function TopBar() {
  const { repositories, current, currentId, setCurrent, addRepository } = useRepository();

  const items: MenuItem[] = repositories.map((repo) => ({
    id: repo.id,
    label: repo.name,
    detail: repo.path,
    selected: repo.id === currentId,
    icon: <FolderOpen size={15} weight="regular" />,
    onSelect: () => setCurrent(repo.id),
  }));

  const footer: MenuItem = {
    id: "add-repository",
    label: "Add repository…",
    icon: <Plus size={15} weight="regular" />,
    onSelect: addRepository,
  };

  return (
    <header className="flex h-9 shrink-0 items-center border-b border-border bg-surface px-2">
      <Menu
        trigger={() => <RepoTrigger name={current.name} />}
        items={items}
        footer={footer}
        minWidth={240}
      />
    </header>
  );
}

function RepoTrigger({ name }: { name: string }) {
  return (
    <span
      title="Switch repository"
      className="flex items-center gap-1.5 rounded-button px-2 py-1 text-[13px] text-text transition-colors hover:bg-white/5"
    >
      <FolderOpen size={15} weight="regular" className="text-text-muted" />
      <span className="max-w-55 truncate font-medium">{name}</span>
      <CaretDown size={12} className="text-text-muted" />
    </span>
  );
}
