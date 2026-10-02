import {
  BookOpenCheck,
  FolderGit2,
  GitPullRequest,
  LayoutDashboard,
  Plug,
  Settings,
  Wallet,
  type LucideIcon,
} from 'lucide-react';

export interface NavItem {
  href: string;
  label: string;
  icon: LucideIcon;
}

export const NAV_ITEMS: NavItem[] = [
  { href: '/', label: 'Dashboard', icon: LayoutDashboard },
  { href: '/repositories', label: 'Repositories', icon: FolderGit2 },
  { href: '/pull-requests', label: 'Pull Requests', icon: GitPullRequest },
  { href: '/rules', label: 'Rules', icon: BookOpenCheck },
  { href: '/integrations', label: 'Integrations', icon: Plug },
  { href: '/usage', label: 'Usage', icon: Wallet },
  { href: '/settings', label: 'Settings', icon: Settings },
];

export function isActive(pathname: string, href: string): boolean {
  return href === '/' ? pathname === '/' : pathname === href || pathname.startsWith(`${href}/`);
}
