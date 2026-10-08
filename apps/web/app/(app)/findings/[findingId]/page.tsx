import type { Metadata } from 'next';
import { FindingDetailView } from '@/components/finding/FindingDetailView';

export const metadata: Metadata = { title: 'Finding' };

export default async function FindingPage({ params }: { params: Promise<{ findingId: string }> }) {
  const { findingId } = await params;
  return <FindingDetailView findingId={findingId} />;
}
