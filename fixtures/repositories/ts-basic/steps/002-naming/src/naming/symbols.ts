export const iterable = {
  [Symbol.iterator]() {
    return [1, 2, 3];
  },
  [Symbol.asyncIterator]() {
    return null;
  },
};

const dynamic = "dynamic";
export const computed = {
  [dynamic]: 1,
};