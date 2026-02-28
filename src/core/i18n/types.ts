export type MessageTree = { [key: string]: string | MessageTree };

export type DotPath<T> = T extends string
  ? never
  : {
      [K in Extract<keyof T, string>]: T[K] extends string
        ? K
        : T[K] extends MessageTree
          ? `${K}.${DotPath<T[K]>}`
          : never;
    }[Extract<keyof T, string>];

export type TranslateParams = Record<string, string | number>;