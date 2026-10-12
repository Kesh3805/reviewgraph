import type { Metadata } from 'next';
import { Suspense } from 'react';
import { PullsView } from '@/components/pulls/PullsView';
import { Loading } from '@/components/ui/states';

export const metadata: Metadata = { title: 'Pull Requests' };

export default async function RepositoryPullsPage({
  params,
}: {
  params: Promise<{ repoId: string }>;
}) {
  const { repoId } = await params;
  return (
    <Suspense fallback={<Loading label="Loading pull requests" />}>
      <PullsView repoId={repoId} />
    </Suspense>
  );
}
