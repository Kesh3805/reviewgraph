import type { Metadata } from 'next';
import { RepoOverview } from '@/components/repositories/RepoOverview';

export const metadata: Metadata = { title: 'Repository' };

export default async function RepositoryOverviewPage({
  params,
}: {
  params: Promise<{ repoId: string }>;
}) {
  const { repoId } = await params;
  return <RepoOverview repoId={repoId} />;
}
