import { useCallback, useEffect, useState } from "react";
import { Outlet, useOutletContext, useParams, useSearchParams } from "react-router-dom";
import type { Character, ChatAppearanceSettings } from "../../../core/storage/schemas";
import {
  createDefaultChatAppearanceSettings,
  mergeChatAppearance,
} from "../../../core/storage/schemas";
import { listCharacters, readSettings } from "../../../core/storage/repo";
import { SETTINGS_UPDATED_EVENT } from "../../../core/storage/repo";
import { preloadImageUrls, useImageDataState } from "../../hooks/useImageData";
import { useChatController, type ChatController } from "./hooks/useChatController";
import {
  analyzeImageBrightness,
  computeChatTheme,
  getDefaultThemeSync,
  type ThemeColors,
} from "../../../core/utils/imageAnalysis";

export interface ChatLayoutContext {
  character: Character | null;
  characterLoading: boolean;
  backgroundImageData: string | undefined;
  backgroundImageLoading: boolean;
  isBackgroundLight: boolean;
  theme: ThemeColors;
  chatAppearance: ChatAppearanceSettings;
  chatController: ChatController;
  reloadCharacter: () => void;
}

export function useChatLayoutContext() {
  return useOutletContext<ChatLayoutContext>();
}

export function ChatLayout() {
  const { characterId } = useParams<{ characterId: string }>();
  const [searchParams] = useSearchParams();
  const [character, setCharacter] = useState<Character | null>(null);
  const [loading, setLoading] = useState(true);
  const [loadCount, setLoadCount] = useState(0);
  const sessionId = searchParams.get("sessionId") || undefined;

  const [bgBrightness, setBgBrightness] = useState<number | null>(null);
  const [chatAppearance, setChatAppearance] = useState<ChatAppearanceSettings>(
    createDefaultChatAppearanceSettings(),
  );
  const [theme, setTheme] = useState<ThemeColors>(getDefaultThemeSync());
  const chatController = useChatController(characterId, { sessionId });

  useEffect(() => {
    let cancelled = false;
    (async () => {
      if (!characterId) {
        setLoading(false);
        setCharacter(null);
        return;
      }
      try {
        setLoading(true);
        const [chars, settings] = await Promise.all([listCharacters(), readSettings()]);
        const match = chars.find((c) => c.id === characterId) ?? null;
        if (!cancelled) {
          setCharacter(match);
          const globalAppearance =
            settings.advancedSettings?.chatAppearance ?? createDefaultChatAppearanceSettings();
          const merged = mergeChatAppearance(globalAppearance, match?.chatAppearance);
          setChatAppearance(merged);
        }
      } catch (err) {
        console.error("ChatLayout: failed to load character", err);
        if (!cancelled) setCharacter(null);
      } finally {
        if (!cancelled) setLoading(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [characterId, loadCount]);

  useEffect(() => {
    const onSettingsUpdated = () => {
      setLoadCount((c) => c + 1);
    };
    window.addEventListener(SETTINGS_UPDATED_EVENT, onSettingsUpdated);
    return () => window.removeEventListener(SETTINGS_UPDATED_EVENT, onSettingsUpdated);
  }, []);

  const reloadCharacter = useCallback(() => {
    setLoadCount((c) => c + 1);
  }, []);

  const backgroundCharacter = chatController.character;
  const effectiveSceneId = chatController.session
    ? (chatController.session.selectedSceneId ?? backgroundCharacter?.defaultSceneId ?? null)
    : null;
  const effectiveSceneBackgroundImagePath = backgroundCharacter?.scenes.find(
    (scene) => scene.id === effectiveSceneId,
  )?.backgroundImagePath;
  const effectiveBackgroundImagePath =
    chatController.session?.backgroundImagePath ??
    effectiveSceneBackgroundImagePath ??
    (chatController.session ? backgroundCharacter?.backgroundImagePath : undefined);
  const {
    imageUrl: resolvedBackgroundImageData,
    loading: resolvedBackgroundImageLoading,
  } = useImageDataState(effectiveBackgroundImagePath);
  const activeBackgroundImageData = effectiveBackgroundImagePath
    ? resolvedBackgroundImageData
    : undefined;
  const backgroundImageLoading =
    !!effectiveBackgroundImagePath && !activeBackgroundImageData && resolvedBackgroundImageLoading;

  useEffect(() => {
    void preloadImageUrls([
      chatController.session?.backgroundImagePath,
      backgroundCharacter?.backgroundImagePath,
      ...(backgroundCharacter?.scenes.map((scene) => scene.backgroundImagePath) ?? []),
    ]);
  }, [backgroundCharacter, chatController.session?.backgroundImagePath]);

  useEffect(() => {
    let mounted = true;

    if (!activeBackgroundImageData) {
      setBgBrightness(null);
      computeChatTheme(chatAppearance, null).then((t) => {
        if (mounted) setTheme(t);
      });
      return () => {
        mounted = false;
      };
    }

    analyzeImageBrightness(activeBackgroundImageData).then((brightness) => {
      if (!mounted) return;
      setBgBrightness(brightness);
      computeChatTheme(chatAppearance, brightness).then((t) => {
        if (mounted) setTheme(t);
      });
    });

    return () => {
      mounted = false;
    };
  }, [activeBackgroundImageData, chatAppearance]);

  const isBackgroundLight = bgBrightness !== null && bgBrightness > 127.5;

  const ctx: ChatLayoutContext = {
    character,
    characterLoading: loading,
    backgroundImageData: activeBackgroundImageData,
    backgroundImageLoading,
    isBackgroundLight,
    theme,
    chatAppearance,
    chatController,
    reloadCharacter,
  };

  return (
    <>
      {activeBackgroundImageData && (
        <div
          className="pointer-events-none fixed inset-0 z-0"
          style={{
            backgroundImage: `url(${activeBackgroundImageData})`,
            backgroundSize: "cover",
            backgroundPosition: "center",
            backgroundRepeat: "no-repeat",
          }}
        />
      )}
      {activeBackgroundImageData && chatAppearance.backgroundBlur > 0 && (
        <div
          className="pointer-events-none fixed inset-0 z-0 transform-gpu backdrop-blur-md will-change-opacity"
          style={{
            opacity: Math.min(1, chatAppearance.backgroundBlur / 20),
            backgroundColor: "rgba(0, 0, 0, 0.01)",
          }}
        />
      )}
      {activeBackgroundImageData && chatAppearance.backgroundDim > 0 && (
        <div
          className="pointer-events-none fixed inset-0 z-0"
          style={{
            backgroundColor: `rgba(0, 0, 0, ${chatAppearance.backgroundDim / 100})`,
          }}
        />
      )}
      <Outlet context={ctx} />
    </>
  );
}
