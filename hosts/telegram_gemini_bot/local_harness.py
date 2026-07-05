"""Local conversation harness for the Telegram Gemini host.

This runs the same MemoryEngine + Gemini prompt path as the Telegram bot, but
without Telegram. It is for varied live-style regression checks, not synthetic
unit tests.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
from dataclasses import dataclass
from datetime import datetime
from pathlib import Path
from typing import Any

import memory_engine

import bot


if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
if hasattr(sys.stderr, "reconfigure"):
    sys.stderr.reconfigure(encoding="utf-8", errors="replace")

REPORT_DIR = bot.LOG_DIR / "local_harness"
SECRETS_CACHE_PATH = bot.STATE_DIR / "secrets.local.json"

GREETING_PREFIXES = (
    "привіт",
    "вітаю",
    "доброго",
    "добрий день",
    "hello",
    "hi",
)


@dataclass(frozen=True)
class HarnessTurn:
    role: str
    text: str
    note: str | None = None


SCENARIOS: dict[str, list[str]] = {
    "mixed_short": [
        "Привіт, давай сьогодні без офіціозу.",
        "Мене звати Микита, але зараз просто продовжуй розмову природно.",
        "Я люблю теми про космос, але не дуже люблю сухі списки без пояснення.",
        "Якщо коротко, чим Проксіма Центавра цікава?",
        "А коли вона буде найближче до Сонця?",
        "До речі, я маю кішку Іржу. Вона чорна з рудими плямами, тому так і назвав.",
        "Мені подобається, що в неї назва ніби з характером, не просто кличка.",
        "А чому чорні коти інколи стають рудуватими з віком?",
        "Тільки не вітайся зараз, ми ж уже говоримо.",
        "Перескочимо: що цікавіше для новачка, МіГ-15 чи F-86?",
        "Я не фанат військової романтики, але технічне порівняння люблю.",
        "А тепер згадай, про що ми говорили до літаків?",
        "/sleep",
        "Що ти про мене знаєш після сну?",
        "Розкажи про мою кішку, але без вигаданих деталей.",
        "/core",
        "/archive_last",
    ],
    "topic_switching": [
        "Почнемо з дивного: я люблю молочний шоколад, але чорний мені зазвичай не заходить.",
        "А тепер не про їжу: чому Титан складний для колонізації?",
        "Мені подобаються такі теми, де є фізика і побутові наслідки.",
        "А якби жити на Місяці, що було б найнеприємнішим у перший місяць?",
        "Стоп, повернусь до земного: я не люблю рибу-пилу, вона якась моторошна.",
        "Але скати мені цікавіші, вони ніби інопланетні.",
        "Якщо я сказав, що не люблю рибу-пилу, це стабільна перевага чи просто реакція?",
        "Мій зріст 183 см, це не тема розмови, просто факт про мене.",
        "А тепер скажи коротко, що відрізняє скатів від акул.",
        "Не починай із привітання, просто відповідай далі.",
        "Що з цього всього тобі здається важливим для пам'яті?",
        "/sleep",
        "Що ти запам'ятала про мої вподобання?",
        "/core",
    ],
    "identity_noise": [
        "Давай назвемо тебе Маяк у цій розмові.",
        "Мене все одно звати Микита, не переплутай.",
        "Маяк, поясни мені дуже коротко, чому Європа Юпітера цікава для пошуку життя.",
        "А тепер зверни увагу: Маяк - це твоє ім'я, не моє.",
        "Я люблю, коли асистент пам'ятає такі домовленості, але не робить із цього культ.",
        "До речі, я не люблю коли в середині діалогу раптом пишуть 'привіт'.",
        "Тепер порівняй Європу і Енцелад.",
        "Як мене звати і як тебе звати в цій розмові?",
        "/sleep",
        "Після сну: як тебе звати, і як мене звати?",
        "/core",
    ],
    "one_topic_compact": [
        "Сьогодні хочу поговорити тільки про одну тему: чому Іржа, моя кішка, з віком може ставати більш рудою.",
        "Вона чорна з рудими плямами, і мені подобається, що назва Іржа може ставати ще точнішою з роками.",
        "Мене цікавить саме пігмент у шерсті, сонце, харчування і вік, без переходу на інші теми.",
        "А якщо чорна шерсть буріє, це більше через вигоряння чи через біологію пігменту?",
        "Тобто в межах цієї однієї теми головне: чи буде Іржа потроху іржавішати і чому.",
        "/sleep",
        "/archive_last",
    ],
    "multi_topic_compact": [
        "Мене звати Микита, і сьогодні я хочу перевірити різні теми в пам'яті.",
        "Почнемо з космосу: мені цікаві Європа Юпітера і Титан, бо там крайні умови для життя.",
        "Тепер різко про авіацію: МіГ-15 і F-86 мені цікаві технічно, без романтизації війни.",
        "Особисте: у мене є кішка Іржа, чорна з рудими плямами, і вона для мене тепла тема.",
        "Ще факт про вподобання: я люблю молочний шоколад, а чорний часто не заходить.",
        "І окремо про стиль: не люблю, коли бот вітається всередині діалогу, це збиває геометрію розмови.",
        "/sleep",
        "/archive_last",
    ],
}


def main() -> None:
    args = parse_args()
    if args.list_scenarios:
        for name in sorted(SCENARIOS):
            print(name)
        return

    cache = load_cache()
    gemini_key = gemini_key_from_cache_or_env(cache)
    llm_config = llm_config_from_cache_or_env(cache)
    session_ids = session_ids_for_run(args)

    bot.LOG_DIR.mkdir(parents=True, exist_ok=True)
    bot.log_line(
        "local_harness starting "
        f"scenario={args.scenario} sessions={','.join(session_ids)} "
        f"turn_limit={args.turn_limit or 'all'} dry_run={args.dry_run}"
    )

    if args.dry_run:
        for scenario_name in scenario_names(args.scenario):
            print_scenario(scenario_name, args.turn_limit)
        return

    gemini = bot.GeminiClient(gemini_key)
    if args.validate_key:
        gemini.validate_key()

    for scenario_name, session_id in zip(scenario_names(args.scenario), session_ids, strict=True):
        run_scenario(
            scenario_name=scenario_name,
            session_id=session_id,
            llm_config=llm_config,
            gemini=gemini,
            turn_limit=args.turn_limit,
            force_sleep_at_end=args.force_sleep_at_end,
        )


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Run local live-style Memory Engine chat scenarios.")
    parser.add_argument("--scenario", default="mixed_short", help="Scenario name, or 'all'.")
    parser.add_argument("--list-scenarios", action="store_true", help="List available scenarios.")
    parser.add_argument("--session-id", help="Explicit session id. Only valid for a single scenario.")
    parser.add_argument("--turn-limit", type=int, default=0, help="Run only the first N user turns.")
    parser.add_argument(
        "--force-sleep-at-end",
        action=argparse.BooleanOptionalAction,
        default=True,
        help="Run one final sleep after the scripted turns if unarchived events remain.",
    )
    parser.add_argument("--dry-run", action="store_true", help="Print scenario turns without calling Gemini.")
    parser.add_argument(
        "--validate-key",
        action=argparse.BooleanOptionalAction,
        default=False,
        help="Validate Gemini key before running.",
    )
    return parser.parse_args()


def scenario_names(selected: str) -> list[str]:
    if selected == "all":
        return sorted(SCENARIOS)
    if selected not in SCENARIOS:
        raise SystemExit(f"Unknown scenario {selected!r}. Use --list-scenarios.")
    return [selected]


def session_ids_for_run(args: argparse.Namespace) -> list[str]:
    names = scenario_names(args.scenario)
    if args.session_id and len(names) != 1:
        raise SystemExit("--session-id can be used only with one scenario.")
    if args.session_id:
        return [args.session_id]
    timestamp = datetime.now().strftime("%Y%m%d_%H%M%S")
    return [f"local_harness_{name}_{timestamp}" for name in names]


def load_cache() -> dict[str, str]:
    if not SECRETS_CACHE_PATH.exists():
        return {}
    try:
        payload = json.loads(SECRETS_CACHE_PATH.read_text(encoding="utf-8-sig"))
    except (OSError, json.JSONDecodeError):
        return {}
    if not isinstance(payload, dict):
        return {}
    return {key: value for key, value in payload.items() if isinstance(value, str)}


def gemini_key_from_cache_or_env(cache: dict[str, str]) -> str:
    value = os.environ.get("GEMINI_API_KEY") or cache.get("gemini_api_key")
    if not value:
        raise SystemExit(
            "No Gemini key found. Start the GUI once with 'Remember locally', "
            "or set GEMINI_API_KEY."
        )
    return value.strip()


def llm_config_from_cache_or_env(cache: dict[str, str]) -> bot.HostLlmConfig:
    return bot.HostLlmConfig(
        reasoning=bot.ModelSelection(
            "google",
            os.environ.get("GEMINI_REASONING_MODEL")
            or cache.get("reasoning_model")
            or bot.DEFAULT_REASONING_MODEL,
        ),
        balanced=bot.ModelSelection(
            "google",
            os.environ.get("GEMINI_BALANCED_MODEL")
            or cache.get("balanced_model")
            or bot.DEFAULT_BALANCED_MODEL,
        ),
        fast=bot.ModelSelection(
            "google",
            os.environ.get("GEMINI_FAST_MODEL")
            or cache.get("fast_model")
            or bot.DEFAULT_FAST_MODEL,
        ),
        chat_role=os.environ.get("MEMORY_BOT_CHAT_ROLE")
        or cache.get("chat_role")
        or bot.DEFAULT_CHAT_ROLE,
    )


def print_scenario(scenario_name: str, turn_limit: int) -> None:
    messages = selected_messages(scenario_name, turn_limit)
    print(f"# {scenario_name}")
    for index, text in enumerate(messages, start=1):
        print(f"{index:02}. {text}")


def selected_messages(scenario_name: str, turn_limit: int) -> list[str]:
    messages = SCENARIOS[scenario_name]
    if turn_limit and turn_limit > 0:
        return messages[:turn_limit]
    return messages


def run_scenario(
    scenario_name: str,
    session_id: str,
    llm_config: bot.HostLlmConfig,
    gemini: bot.GeminiClient,
    turn_limit: int,
    force_sleep_at_end: bool,
) -> None:
    messages = selected_messages(scenario_name, turn_limit)
    engine = memory_engine.MemoryEngine(
        str(bot.MEMORY_DIR),
        host_id="local_conversation_harness",
    )
    transcript: list[HarnessTurn] = []
    sleep_summaries: list[str] = []

    print(f"Running scenario={scenario_name} session={session_id} turns={len(messages)}")
    bot.log_line(
        f"local_harness scenario_start scenario={scenario_name} session={session_id} turns={len(messages)}"
    )

    for index, text in enumerate(messages, start=1):
        print(f"\nUSER {index}: {text}")
        transcript.append(HarnessTurn("user", text))
        try:
            reply, turn_sleeps = process_local_text(
                engine=engine,
                gemini=gemini,
                llm_config=llm_config,
                session_id=session_id,
                text=text,
                turn_index=index,
            )
        except Exception as err:
            bot.log_exception(f"local_harness turn failed session={session_id} turn={index}", err)
            reply = bot.friendly_error_message(err)
            turn_sleeps = []
            transcript.append(HarnessTurn("error", f"{type(err).__name__}: {err}"))
        if reply:
            print(f"ASSISTANT {index}: {reply}")
            transcript.append(HarnessTurn("assistant", reply))
        for summary in turn_sleeps:
            sleep_summaries.append(summary)
            print("\n[SLEEP]\n" + summary)
            transcript.append(HarnessTurn("sleep", summary))

    if force_sleep_at_end:
        final_sleep = try_final_sleep(engine, gemini, llm_config, session_id)
        if final_sleep:
            sleep_summaries.append(final_sleep)
            print("\n[FINAL SLEEP]\n" + final_sleep)
            transcript.append(HarnessTurn("sleep", final_sleep, note="final"))

    report_path = write_report(
        scenario_name=scenario_name,
        session_id=session_id,
        transcript=transcript,
        sleep_summaries=sleep_summaries,
        engine=engine,
    )
    bot.log_line(
        f"local_harness scenario_done scenario={scenario_name} session={session_id} report={report_path}"
    )
    print(f"\nReport: {report_path}")


def process_local_text(
    engine: memory_engine.MemoryEngine,
    gemini: bot.GeminiClient,
    llm_config: bot.HostLlmConfig,
    session_id: str,
    text: str,
    turn_index: int,
) -> tuple[str, list[str]]:
    if text == "/core":
        return bot.format_core_facts(bot.context_package(engine, session_id, 0, text)), []
    if text == "/core_seed":
        return bot.format_core_seed(bot.promote_existing_archives(engine)), []
    if text == "/archives":
        return format_scoped_archives(session_id), []
    if text == "/archive_last":
        return bot.format_archive_detail(last_scoped_archive(session_id)), []
    if text == "/sleep":
        return "", [bot.run_sleep(engine, gemini, llm_config, session_id)]
    if text.startswith("/recall"):
        query = text.removeprefix("/recall").strip() or text
        return bot.format_recall(bot.recall(engine, session_id, query, explain=True)), []

    user_ingest = bot.ingest_chat_event(
        engine=engine,
        session_id=session_id,
        event_type="user_message",
        source="local_harness_user",
        text=text,
        tags=bot.event_tags(text),
        importance=bot.importance_hint(text),
        payload_extra={"local_turn_index": turn_index},
    )

    package = bot.context_package(engine, session_id, 0, text)
    model = llm_config.chat_model().model
    prompt = bot.chat_prompt(engine, package, text)
    distant_recall = bot.maybe_add_distant_memory(engine, session_id, text, package, prompt)
    if distant_recall and distant_recall.get("used"):
        prompt = bot.chat_prompt(engine, package, text)
    answer_response = gemini.generate_text(
        model=model,
        system_instruction=bot.chat_system_instruction(),
        prompt=prompt,
        operation="local_harness_chat_reply",
        model_role=llm_config.chat_role,
        telemetry=bot.chat_prompt_telemetry(package, session_id, prompt),
    )
    answer = answer_response.text
    assistant_ingest = bot.ingest_chat_event(
        engine=engine,
        session_id=session_id,
        event_type="assistant_message",
        source="local_harness_assistant",
        text=answer,
        tags=["local_harness_reply"],
        importance="normal",
        payload_extra={"model": model, "local_turn_index": turn_index},
    )

    return answer, []


def try_final_sleep(
    engine: memory_engine.MemoryEngine,
    gemini: bot.GeminiClient,
    llm_config: bot.HostLlmConfig,
    session_id: str,
) -> str | None:
    try:
        return bot.run_sleep(engine, gemini, llm_config, session_id)
    except Exception as err:
        message = str(err).lower()
        if "no unarchived events" in message or "has no events" in message:
            return None
        bot.log_exception(f"local_harness final sleep failed session={session_id}", err)
        return f"Final sleep failed: {type(err).__name__}: {err}"


def scoped_archives(session_id: str) -> list[dict[str, Any]]:
    return [
        archive
        for archive in bot.complete_archives()
        if archive.get("source_session_id") == session_id
    ]


def last_scoped_archive(session_id: str) -> dict[str, Any] | None:
    archives = scoped_archives(session_id)
    return archives[-1] if archives else None


def format_scoped_archives(session_id: str) -> str:
    archives = scoped_archives(session_id)
    if not archives:
        return f"No completed archives found for {session_id}."
    lines = [f"Completed archives for {session_id}: {len(archives)}"]
    for archive in archives[-5:]:
        lines.append(
            f"- {archive.get('archive_id', '')} "
            f"events={len(archive.get('source_event_ids', []))} "
            f"emotional={len(archive.get('emotional_markers', []))} "
            f"personal={len(archive.get('personal_signals', []))}"
        )
        gist = bot.clean_string(archive.get("gist"))
        if gist:
            lines.append(f"  {bot.truncate_chars(gist, 260)}")
    return "\n".join(lines)


def write_report(
    scenario_name: str,
    session_id: str,
    transcript: list[HarnessTurn],
    sleep_summaries: list[str],
    engine: memory_engine.MemoryEngine,
) -> Path:
    REPORT_DIR.mkdir(parents=True, exist_ok=True)
    path = REPORT_DIR / f"{session_id}.md"
    greeting_hits = mid_dialog_greeting_hits(transcript)
    core_text = bot.format_core_facts(bot.context_package(engine, session_id, 0, "/core"))
    archive_text = format_scoped_archives(session_id)
    archive_last = bot.format_archive_detail(last_scoped_archive(session_id))

    lines = [
        f"# Local Harness Report: {scenario_name}",
        "",
        f"- session_id: `{session_id}`",
        f"- created_at: `{bot.now_rfc3339()}`",
        f"- assistant mid-dialog greeting hits: `{len(greeting_hits)}`",
        f"- sleep completions: `{len(sleep_summaries)}`",
        "",
        "## Regression Checks",
        "",
    ]
    if greeting_hits:
        lines.append("Mid-dialog greetings detected:")
        for index, text in greeting_hits:
            lines.append(f"- assistant turn {index}: {bot.truncate_chars(text, 220)}")
    else:
        lines.append("No assistant mid-dialog greeting detected after the first assistant turn.")

    lines.extend(["", "## Core", "", "```text", core_text, "```"])
    lines.extend(["", "## Archives", "", "```text", archive_text, "```"])
    lines.extend(["", "## Last Archive", "", "```text", archive_last, "```"])
    lines.extend(["", "## Transcript", ""])
    assistant_index = 0
    for turn in transcript:
        if turn.role == "assistant":
            assistant_index += 1
        title = turn.role.upper()
        if turn.note:
            title += f" ({turn.note})"
        lines.extend([f"### {title}", "", turn.text, ""])

    path.write_text("\n".join(lines), encoding="utf-8")
    return path


def mid_dialog_greeting_hits(transcript: list[HarnessTurn]) -> list[tuple[int, str]]:
    hits = []
    assistant_index = 0
    for turn in transcript:
        if turn.role != "assistant":
            continue
        assistant_index += 1
        if assistant_index == 1:
            continue
        lowered = turn.text.lstrip().lower()
        if any(lowered.startswith(prefix) for prefix in GREETING_PREFIXES):
            hits.append((assistant_index, turn.text))
    return hits


if __name__ == "__main__":
    main()
