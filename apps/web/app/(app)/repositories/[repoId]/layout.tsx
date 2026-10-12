import type { ReactNode } from 'react';
import { RepoHeader } from '@/components/repositories/RepoTabs';

export default async function RepositoryLayout({
  children,
  params,
}: {
  children: ReactNode;
  params: Promise<{ repoId: string }>;
}) {
  const { repoId } = await params;
  return (
    <div className="space-y-6">
      <RepoHeader repoId={repoId} />
      {children}
    </div>
  );
}
