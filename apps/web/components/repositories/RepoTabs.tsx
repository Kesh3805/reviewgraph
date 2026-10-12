'use client';

import { useQuery } from '@tanstack/react-query';
import Link from 'next/link';
import { usePathname } from 'next/navigation';
import { Badge } from '@/components/ui/badge';
import { repositoryQuery } from '@/lib/queries';
import { cn } from '@/lib/utils';

export const REPO_TABS = [
  { segment: '', label: 'Overview' },
  { segment: 'intelligence', label: 'Intelligence' },
  { segment: 'profile', label: 'Profile' },
  { segment: 'pulls', label: 'Pull Requests' },
  { segment: 'rules', label: 'Rules' },
  { segment: 'graph', label: 'Graph' },
] as const;

export function activeTab(pathname: string, repoId: string): string {
  const base = `/repositories/${repoId}`;
  const rest = pathname.startsWith(base) ? pathname.slice(base.length).replace(/^\//, '') : '';
  return rest.split('/')[0] ?? '';
}

/** Repository name plus the Overview · Intelligence · Profile · Pull Requests · Rules · Graph tabs. */
export function RepoHeader({ repoId }: { repoId: string }) {
  const pathname = usePathname();
  const repo = useQuery({ ...repositoryQuery(repoId), throwOnError: false });
  const current = activeTab(pathname, repoId);

  return (
    <div className="space-y-4">
      <div className="flex flex-wrap items-center gap-3">
        <Link href="/repositories" className="text-sm text-muted-foreground hover:underline">
          Repositories
        </Link>
        <span className="text-muted-foreground">/</span>
        <h1 className="text-2xl font-semibold">{repo.data?.full_name ?? 'Repository'}</h1>
        {repo.data && !repo.data.enabled && <Badge variant="muted">Disabled</Badge>}
      </div>
      <nav aria-label="Repository" className="flex gap-1 overflow-x-auto border-b">
        {REPO_TABS.map(({ segment, label }) => {
          const active = current === segment;
          return (
            <Link
              key={label}
              href={`/repositories/${repoId}${segment ? `/${segment}` : ''}`}
              aria-current={active ? 'page' : undefined}
              className={cn(
                '-mb-px border-b-2 px-3 py-2 text-sm font-medium whitespace-nowrap',
                active
                  ? 'border-primary text-foreground'
                  : 'border-transparent text-muted-foreground hover:text-foreground',
              )}
            >
              {label}
            </Link>
          );
        })}
      </nav>
    </div>
  );
}
