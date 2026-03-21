export interface AvatarLibrarySelectionPayload {
  filePath: string;
}

const AVATAR_LIBRARY_SELECTION_PREFIX = "avatar-library-selection:";

export function buildAvatarLibrarySelectionKey(returnPath: string): string {
  return `${AVATAR_LIBRARY_SELECTION_PREFIX}${returnPath}`;
}
