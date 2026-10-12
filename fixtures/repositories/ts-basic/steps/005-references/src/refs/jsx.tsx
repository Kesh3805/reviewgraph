import { Button } from './button';
import * as UI from './ui';

export function View() {
  return (
    <div>
      <Button label="x" />
      <UI.Panel>text</UI.Panel>
    </div>
  );
}
