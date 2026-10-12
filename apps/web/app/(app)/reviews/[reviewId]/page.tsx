import type { Metadata } from 'next';
import { ReviewDetailView } from '@/components/review/ReviewDetailView';

export const metadata: Metadata = { title: 'Review' };

export default async function ReviewPage({ params }: { params: Promise<{ reviewId: string }> }) {
  const { reviewId } = await params;
  return <ReviewDetailView reviewId={reviewId} />;
}
