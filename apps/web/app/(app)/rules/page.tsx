import type { Metadata } from 'next';
import { OrgRulesIndex } from '@/components/rules/OrgRulesIndex';

export const metadata: Metadata = { title: 'Rules' };

export default function RulesPage() {
  return <OrgRulesIndex />;
}
