import type { Metadata } from 'next';
import { Suspense } from 'react';
import { GraphExplorer } from '@/components/graph/GraphExplorer';
import { Loading } from '@/components/ui/states';

export const metadata: Metadata = { title: 'Graph explorer' };

export default async function GraphPage({ params }: { params: Promise<{ repoId: string }> }) {
  const { repoId } = await params;
  return (
    <Suspense fallback={<Loading label="Loading explorer" />}>
      <GraphExplorer repoId={repoId} />
    </Suspense>
  );
}
