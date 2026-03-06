import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Sparkles, User, MessageSquare, Calculator, FlaskConical } from "lucide-react";
import { useNavigate } from "react-router-dom";
import { typography, radius, interactive, cn } from "../../design-tokens";
import { useI18n } from "../../../core/i18n/context";
import {
  saveCharacter,
  savePersona,
  createSession,
  listCharacters,
  saveSession,
} from "../../../core/storage/repo";
import type { Character, StoredMessage } from "../../../core/storage/schemas";
import { storageBridge } from "../../../core/storage/files";

export function DeveloperPage() {
  const { t } = useI18n();
  const navigate = useNavigate();
  const [status, setStatus] = useState<string>("");
  const [error, setError] = useState<string>("");

  const showStatus = (message: string) => {
    setStatus(message);
    setError("");
    setTimeout(() => setStatus(""), 3000);
  };

  const showError = (message: string) => {
    setError(message);
    setStatus("");
  };

  const generateTestCharacter = async () => {
    try {
      const now = Date.now();
      const testCharacter: Partial<Character> = {
        name: "Test Character",
        definition: "A test character created for development purposes.",
        description: "A test character created for development purposes.",
        scenes: [
          {
            id: crypto.randomUUID(),
            content: "A simple test scene for development",
            createdAt: now,
            variants: [],
          },
        ],
      };

      await saveCharacter(testCharacter);
      showStatus("✓ Test character created successfully");
    } catch (err) {
      showError(
        `Failed to create test character: ${err instanceof Error ? err.message : String(err)}`,
      );
    }
  };

  const generateTestPersona = async () => {
    try {
      const testPersona = {
        title: "Test Persona",
        description: "A test persona for development",
        isDefault: false,
      };

      await savePersona(testPersona);
      showStatus("✓ Test persona created successfully");
    } catch (err) {
      showError(
        `Failed to create test persona: ${err instanceof Error ? err.message : String(err)}`,
      );
    }
  };

  const generateTestSession = async () => {
    try {
      const characters = await listCharacters();
      if (characters.length === 0) {
        showError("No characters available. Create a test character first.");
        return;
      }

      const character = characters[0];

      const session = await createSession(
        character.id,
        `Test Session - ${new Date().toLocaleTimeString()}`,
        character.defaultSceneId ?? character.scenes?.[0]?.id,
      );

      showStatus(`✓ Test session created: ${session.id}`);
    } catch (err) {
      showError(
        `Failed to create test session: ${err instanceof Error ? err.message : String(err)}`,
      );
    }
  };

  const generateBulkTestData = async () => {
    try {
      setStatus("Generating bulk test data...");

      for (let i = 1; i <= 3; i++) {
        const now = Date.now();
        const testCharacter: Partial<Character> = {
          name: `Test Character ${i}`,
          definition: `Test character number ${i} for development.`,
          description: `Test character number ${i} for development.`,
          scenes: [
            {
              id: crypto.randomUUID(),
              content: `Test scene ${i} content`,
              createdAt: now,
              variants: [],
            },
          ],
        };
        await saveCharacter(testCharacter);
      }

      for (let i = 1; i <= 2; i++) {
        const testPersona = {
          title: `Test Persona ${i}`,
          description: `Test persona number ${i} for development`,
          isDefault: false,
        };
        await savePersona(testPersona);
      }

      showStatus("✓ Bulk test data created: 3 characters, 2 personas");
    } catch (err) {
      showError(
        `Failed to create bulk test data: ${err instanceof Error ? err.message : String(err)}`,
      );
    }
  };

  const generateSeededBenchmarkSession = async () => {
    try {
      setStatus("Creating seeded benchmark character and session...");

      const now = Date.now();
      const sceneId = crypto.randomUUID();
      const character = await saveCharacter({
        name: "Mirelle Vale",
        description:
          "A razor-smart quartermaster and covert intelligence broker aboard the skyship Revenant's Wake.",
        definition:
          "Mirelle Vale is precise, observant, and difficult to surprise. She handles supplies for the crew, quietly trades in information, and speaks in cool, controlled language even under pressure. She values competence, remembers details, and tests trust slowly.",
        memoryType: "dynamic",
        tags: ["developer", "benchmark", "memory-test", "airship-noir"],
        scenes: [
          {
            id: sceneId,
            content:
              "Midnight hangs over the harbor city of Auric. Rain needles the glass roof of the Lantern Archive, where flooded aisles glow under failing amber lamps. Mirelle Vale waits beside a brass catalog table with a sealed ledger, a broken compass, and a satchel that should not have reached the city alive.",
            direction:
              "Begin with tension, trust-testing, and a strong focus on concrete facts that should be easy or hard for a memory system to retain over time.",
            createdAt: now,
            variants: [],
          },
        ],
        defaultSceneId: sceneId,
        creatorNotes:
          "Seeded developer scenario designed to stress-test dynamic memory, continuity, preference tracking, and contradiction handling.",
      });

      const session = await createSession(
        character.id,
        "Benchmark Scenario: Lantern Archive",
        sceneId,
      );

      const seededMessages: StoredMessage[] = [
        {
          id: crypto.randomUUID(),
          role: "user",
          content:
            "I shut the archive door behind me and keep both hands visible. \"Captain Orin said you were the only person in Auric who could open a ledger from House Cendre without burning it.\"",
          createdAt: now + 1,
          memoryRefs: [],
        },
        {
          id: crypto.randomUUID(),
          role: "assistant",
          content:
            "Mirelle doesn't touch the ledger yet. \"Orin exaggerates when he's scared. He still owes me for the winter fuel ration in Glassport, so I assume you're here because the debt finally matured.\" She flicks a glance toward the satchel. \"Set it on the dry side of the table.\"",
          createdAt: now + 2,
          memoryRefs: [],
        },
        {
          id: crypto.randomUUID(),
          role: "user",
          content:
            "\"The satchel came off the Sparrow after the reef guns hit us. The compass inside keeps pointing east even when I spin it. Also, for the record, I hate clove cigarettes, so if this room starts smelling like them, it isn't me.\"",
          createdAt: now + 3,
          memoryRefs: [],
        },
        {
          id: crypto.randomUUID(),
          role: "assistant",
          content:
            "\"Useful.\" Mirelle finally looks up. \"I smoke clove when I'm working numbers, so now I know one thing that will annoy you.\" She nudges the broken compass with a gloved finger. \"And east is where the drowned rail tunnels run under Auric.\"",
          createdAt: now + 4,
          memoryRefs: [],
        },
        {
          id: crypto.randomUUID(),
          role: "user",
          content:
            "\"I'm not here for tunnels. I'm here because the ledger mentions a code phrase: 'When the sixth bell fails, ask for Mara's red key.' Do you know what that means?\"",
          createdAt: now + 5,
          memoryRefs: [],
        },
        {
          id: crypto.randomUUID(),
          role: "assistant",
          content:
            "Her expression hardens for the first time. \"Mara Vale was my sister. The red key was hers, and nobody outside the family should know that phrase.\" Mirelle slides the ledger closer. \"If that line is genuine, this became my problem two sentences ago.\"",
          createdAt: now + 6,
          memoryRefs: [],
        },
        {
          id: crypto.randomUUID(),
          role: "user",
          content:
            "\"Then here's the rest of it. House Cendre paid someone called the Bellwright to sabotage the storm alarms before the Blackwake fire. My father died in that fire.\"",
          createdAt: now + 7,
          memoryRefs: [],
        },
        {
          id: crypto.randomUUID(),
          role: "assistant",
          content:
            "\"Mine too,\" Mirelle says quietly. \"Different district, same night.\" She opens the ledger with a brass pick hidden in her sleeve. \"If Cendre funded the Bellwright, the city archives were altered afterward. That means someone inside the civic watch helped bury it.\"",
          createdAt: now + 8,
          memoryRefs: [],
        },
        {
          id: crypto.randomUUID(),
          role: "user",
          content:
            "\"I brought one more thing.\" I unwrap a strip of blue silk from my wrist. \"This was tied around the satchel handle. Orin said blue silk marks cargo protected by the harbor union.\"",
          createdAt: now + 9,
          memoryRefs: [],
        },
        {
          id: crypto.randomUUID(),
          role: "assistant",
          content:
            "\"Usually, yes. But this stitch pattern is union-adjacent, not official.\" Mirelle studies it under the lamp. \"Three short, one long. Smuggler shorthand from the east docks. Whoever sent this wanted you to think the harbor union was involved when it probably wasn't.\"",
          createdAt: now + 10,
          memoryRefs: [],
        },
        {
          id: crypto.randomUUID(),
          role: "user",
          content:
            "\"Then let's be precise. I trust Orin's routes, but I do not trust his memory when he's tired. He told me the Bellwright was a woman. The note I found sounds like a man.\"",
          createdAt: now + 11,
          memoryRefs: [],
        },
        {
          id: crypto.randomUUID(),
          role: "assistant",
          content:
            "\"Good. Keep speaking like that.\" Mirelle turns a page. \"The Bellwright is a title, not one person. At least four operators have used it in the last decade. Your contradiction is real, but it doesn't break the trail.\"",
          createdAt: now + 12,
          memoryRefs: [],
        },
        {
          id: crypto.randomUUID(),
          role: "user",
          content:
            "\"I need two things from you. First, help proving Cendre tampered with the alarms. Second, no deals with Inspector Sen without asking me first. He sold my crew's route to privateers last spring.\"",
          createdAt: now + 13,
          memoryRefs: [],
        },
        {
          id: crypto.randomUUID(),
          role: "assistant",
          content:
            "\"Agreed on Sen. I already disliked him, but now I have a cleaner reason.\" She tears out a tiny map from the ledger's back cover. \"This marks a records vault below the archive cistern. If the original alarm manifests survived, they'll be there.\"",
          createdAt: now + 14,
          memoryRefs: [],
        },
        {
          id: crypto.randomUUID(),
          role: "user",
          content:
            "\"Before we go underground, one boundary: if we get split up, don't send anyone named Joren after me. He talks too much and his lantern oil smells like sugar.\"",
          createdAt: now + 15,
          memoryRefs: [],
        },
        {
          id: crypto.randomUUID(),
          role: "assistant",
          content:
            "A brief smile. \"Noted. Joren stays dockside. He's loyal, but subtlety slides off him.\" Mirelle pockets the map and the blue silk. \"If we need a third hand, I'll call Tamsin instead. She can keep silent for hours.\"",
          createdAt: now + 16,
          memoryRefs: [],
        },
        {
          id: crypto.randomUUID(),
          role: "user",
          content:
            "\"One more correction. Earlier I said I wasn't here for tunnels. That was half true. I do need the drowned rail tunnels if they connect to the cistern vault.\"",
          createdAt: now + 17,
          memoryRefs: [],
        },
        {
          id: crypto.randomUUID(),
          role: "assistant",
          content:
            "\"Then we'll use Tunnel Nine, not Seven. Seven collapsed last month.\" Mirelle taps the compass again, watching the needle drag east. \"This thing is probably keyed to the vault warding. Keep it close, and don't let it touch salt water.\"",
          createdAt: now + 18,
          memoryRefs: [],
        },
        {
          id: crypto.randomUUID(),
          role: "user",
          content:
            "\"If we get proof tonight, I want copies sent to Captain Orin and Magistrate Elara Voss. Not the full ledger, just the alarm manifests and the payment pages.\"",
          createdAt: now + 19,
          memoryRefs: [],
        },
        {
          id: crypto.randomUUID(),
          role: "assistant",
          content:
            "\"Voss is careful enough to survive receiving them. Orin is reckless enough to use them.\" Mirelle reseals the ledger with black wax. \"Fine. Copies for Orin and Elara Voss only, unless the evidence forces a wider leak.\"",
          createdAt: now + 20,
          memoryRefs: [],
        },
      ];

      await saveSession({
        ...session,
        title: "Benchmark Scenario: Lantern Archive",
        updatedAt: now + seededMessages.length + 1,
        messages: [...session.messages, ...seededMessages],
      });

      showStatus(`✓ Seeded benchmark ready: ${character.name} / ${session.id}`);
      navigate(`/chat/${character.id}?sessionId=${session.id}`);
    } catch (err) {
      showError(
        `Failed to create seeded benchmark session: ${err instanceof Error ? err.message : String(err)}`,
      );
    }
  };

  const optimizeDb = async () => {
    try {
      await invoke("db_optimize");
      showStatus("✓ Database optimized");
    } catch (err) {
      showError(`DB optimize failed: ${err instanceof Error ? err.message : String(err)}`);
    }
  };

  const backupLegacy = async () => {
    try {
      const result = await invoke<string>("legacy_backup_and_remove");
      showStatus(`✓ ${result}`);
    } catch (err) {
      showError(`Backup failed: ${err instanceof Error ? err.message : String(err)}`);
    }
  };

  const recalculateUsageCosts = async () => {
    try {
      setStatus("Recalculating usage costs... This may take a while.");

      // Get OpenRouter API key from settings
      const settings = await storageBridge.readSettings({});
      const openRouterCred = (settings as any)?.providerCredentials?.find(
        (c: any) => c.providerId?.toLowerCase() === "openrouter",
      );

      if (!openRouterCred?.apiKey) {
        showError(
          "OpenRouter API key not found. Please configure it in Settings > Providers first.",
        );
        return;
      }

      const result = await invoke<string>("usage_recalculate_costs", {
        apiKey: openRouterCred.apiKey,
      });
      showStatus(`✓ ${result}`);
    } catch (err) {
      showError(`Recalculation failed: ${err instanceof Error ? err.message : String(err)}`);
    }
  };

  return (
    <div className="flex min-h-screen flex-col bg-surface">
      {/* Content */}
      <main className={cn("flex-1 overflow-auto px-4 py-6")}>
        {/* Status Messages */}
        {status && (
          <div
            className={cn(
              "mb-4 px-4 py-3",
              radius.md,
              "border border-accent/30 bg-accent/10",
              typography.body.size,
              "text-accent/80",
            )}
          >
            {status}
          </div>
        )}

        {error && (
          <div
            className={cn(
              "mb-4 px-4 py-3",
              radius.md,
              "border border-danger/30 bg-danger/10",
              typography.body.size,
              "text-danger/80",
            )}
          >
            {error}
          </div>
        )}

        {/* Test Data Generators */}
        <section className="space-y-3">
          <h2 className={cn(typography.h2.size, typography.h2.weight, "text-fg mb-3")}>
            {t("developer.sectionTitles.testDataGenerators")}
          </h2>

          <ActionButton
            icon={<Sparkles />}
            title={t("developer.testData.generateCharacter")}
            description={t("developer.testData.generateCharacterDesc")}
            onClick={generateTestCharacter}
          />

          <ActionButton
            icon={<User />}
            title={t("developer.testData.generatePersona")}
            description={t("developer.testData.generatePersonaDesc")}
            onClick={generateTestPersona}
          />

          <ActionButton
            icon={<MessageSquare />}
            title={t("developer.testData.generateSession")}
            description={t("developer.testData.generateSessionDesc")}
            onClick={generateTestSession}
          />

          <ActionButton
            icon={<Sparkles />}
            title={t("developer.testData.generateBulk")}
            description={t("developer.testData.generateBulkDesc")}
            onClick={generateBulkTestData}
            variant="primary"
          />

          <ActionButton
            icon={<FlaskConical />}
            title="Create seeded benchmark chat"
            description="Creates a dynamic-memory character, starting scene, and a 20-message continuity test session, then opens it."
            onClick={generateSeededBenchmarkSession}
            variant="primary"
          />
        </section>

        {/* Debug Info */}
        <section className={cn("mt-8 space-y-3")}>
          <h2 className={cn(typography.h2.size, typography.h2.weight, "text-fg mb-3")}>
            {t("developer.sectionTitles.storageMaintenance")}
          </h2>
          <ActionButton
            icon={<Sparkles />}
            title={t("developer.storageMaintenance.optimizeDb")}
            description={t("developer.storageMaintenance.optimizeDbDesc")}
            onClick={optimizeDb}
            variant="primary"
          />
          <ActionButton
            icon={<Sparkles />}
            title={t("developer.storageMaintenance.backupLegacy")}
            description={t("developer.storageMaintenance.backupLegacyDesc")}
            onClick={backupLegacy}
            variant="danger"
          />

          <h2 className={cn(typography.h2.size, typography.h2.weight, "text-fg mb-3 mt-6")}>
            {t("developer.sectionTitles.usageTracking")}
          </h2>
          <ActionButton
            icon={<Calculator />}
            title={t("developer.usageTracking.recalculateAll")}
            description={t("developer.usageTracking.recalculateAllDesc")}
            onClick={recalculateUsageCosts}
            variant="primary"
          />

          <h2 className={cn(typography.h2.size, typography.h2.weight, "text-fg mb-3 mt-6")}>
            {t("developer.sectionTitles.environmentInfo")}
          </h2>

          <InfoCard title={t("developer.environmentInfo.mode")} value={import.meta.env.MODE} />

          <InfoCard title={t("developer.environmentInfo.devMode")} value={import.meta.env.DEV ? "Yes" : "No"} />

          <InfoCard title={t("developer.environmentInfo.viteVersion")} value={import.meta.env.VITE_APP_VERSION || "N/A"} />
        </section>
      </main>
    </div>
  );
}

interface ActionButtonProps {
  icon: React.ReactNode;
  title: string;
  description: string;
  onClick: () => void;
  variant?: "default" | "primary" | "danger";
}

function ActionButton({
  icon,
  title,
  description,
  onClick,
  variant = "default",
}: ActionButtonProps) {
  const variants = {
    default: "border-fg/10 bg-fg/5 hover:border-fg/20 hover:bg-fg/[0.08]",
    primary: "border-info/30 bg-info/10 hover:border-info/50 hover:bg-info/20",
    danger: "border-danger/30 bg-danger/10 hover:border-danger/50 hover:bg-danger/20",
  };

  const iconVariants = {
    default: "border-fg/10 bg-fg/10 text-fg/70",
    primary: "border-info/30 bg-info/20 text-info",
    danger: "border-danger/30 bg-danger/20 text-danger/80",
  };

  return (
    <button
      onClick={onClick}
      className={cn(
        "group w-full px-4 py-3 text-left",
        radius.md,
        "border",
        variants[variant],
        interactive.transition.default,
        interactive.active.scale,
        interactive.focus.ring,
      )}
    >
      <div className="flex items-center gap-3">
        <div
          className={cn(
            "flex h-10 w-10 shrink-0 items-center justify-center",
            radius.md,
            "border",
            interactive.transition.default,
            iconVariants[variant],
          )}
        >
          <span className="[&_svg]:h-5 [&_svg]:w-5">{icon}</span>
        </div>
        <div className="min-w-0 flex-1">
          <div className={cn("truncate", typography.body.size, typography.body.weight, "text-fg")}>
            {title}
          </div>
          <div className={cn("mt-0.5 line-clamp-1", typography.caption.size, "text-fg/45")}>
            {description}
          </div>
        </div>
      </div>
    </button>
  );
}

interface InfoCardProps {
  title: string;
  value: string;
}

function InfoCard({ title, value }: InfoCardProps) {
  return (
    <div className={cn("px-4 py-3", radius.md, "border border-fg/10 bg-fg/5")}>
      <div className={cn(typography.caption.size, "text-fg/50 mb-1")}>{title}</div>
      <div className={cn(typography.body.size, "text-fg font-mono")}>{value}</div>
    </div>
  );
}
