import { Injectable } from '@nestjs/common';

export interface Settings {
  locale: string;
  theme: 'light' | 'dark';
}

@Injectable()
export class SettingsService {
  merge(current: Settings, patch: Partial<Settings>): Settings {
    if (current === null || current === undefined) {
      return { locale: 'en', theme: 'light', ...patch };
    }
    return { ...current, ...patch };
  }
}
