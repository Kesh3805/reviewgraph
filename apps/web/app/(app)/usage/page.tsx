import type { Metadata } from 'next';
import { UsageView } from '@/components/usage/UsageView';

export const metadata: Metadata = { title: 'Usage' };

export default function UsagePage() {
  return <UsageView />;
}
