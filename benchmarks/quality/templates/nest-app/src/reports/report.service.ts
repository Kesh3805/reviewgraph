import { Injectable } from '@nestjs/common';

export interface ReportRow {
  label: string;
  owner?: { name: string };
}

@Injectable()
export class ReportService {
  // Builds the header line of the monthly report.
  header(month: string): string {
    return `Monthly report for ${month}`;
  }

  ownerNames(rows: ReportRow[]): string[] {
    return rows.map((r) => r.owner!.name);
  }
}
