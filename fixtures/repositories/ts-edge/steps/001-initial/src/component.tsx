import React from "react";

export function Hello({ name }: { name: string }) {
  return <div className="hello">{name}</div>;
}

export const Generic = <T,>(props: { value: T }) => <span>{String(props.value)}</span>;
