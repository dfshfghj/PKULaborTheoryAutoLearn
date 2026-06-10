import argparse
import base64
import hashlib
import json
import os
import time
import uuid
from urllib.parse import parse_qs, urlparse

import requests
from Crypto.Cipher import AES
from Crypto.Util.Padding import pad


SECRET = "8c1ef35c1a24f94ce6422f3c4b77e19bec2aaec9c0d72251b82ccf40b22561a84c876c19d2cb9a"
KEY = hashlib.sha256(SECRET.encode("utf-8")).digest()
ENDPOINT = "https://mrr.readoor.cn/api/3.1/stat/v1/b/stat31/stat/pStatIf"
SECTIONS_ENDPOINT = "https://api3.readoor.cn/api/3.0/app/v1/dms/spu/sections"
APP_INFO_ENDPOINT = "https://api3.readoor.cn/api/3.1/app/v1/app/info"
IAAA_LOGIN_ENDPOINT = "https://iaaa.pku.edu.cn/iaaa/oauthlogin.do"
NEW_KEY = "AUe#2jE31o90"
REFERER = (
    "https://byyxt.pupedu.cn/550278742975483904/c/pc/viewer"
    "?spu_guid=570535132977475584&group_id=16211&training_id=17296"
    "&project_id=408520&section_guid=570542196487401472"
)
COURSE_CLASS_ID = "16211"
COURSE_TRAIN_ID = "17296"
COURSE_PROJECT_ID = "408520"
COURSE_TASK_GUID = "588410513113788416"
COURSE_SPU_GUID = "570535132977475584"
COURSE_SPU_TYPE = 302
DEFAULT_SECTION_GUID = "570542196487401472"
DEFAULT_COURSEWARE_TYPE = 104
DEFAULT_SECTION_TYPE = 403
DEFAULT_MEDIA_DURATION = 1009.36
DEFAULT_STUDY_TIME = 40
DEFAULT_SEQUENCE_ID = 7
DEFAULT_PLATFORM_CODE = "pweb"


def md5_hex(value: str) -> str:
    return hashlib.md5(value.encode("utf-8")).hexdigest()


def build_signed_form(extra: dict[str, str], *, ts: str | None = None, nonce: str | None = None) -> dict[str, str]:
    ts = ts or str(int(time.time()))
    nonce = nonce or str(uuid.uuid4())
    sign = md5_hex(md5_hex(NEW_KEY + ts + nonce) + ts + nonce)
    return {
        "ts": ts,
        "nonce": nonce,
        "sign": sign,
        "v": "1.0.0",
        **extra,
    }


def build_sections_form(spu_guid: str, module_id: str) -> dict[str, str]:
    return build_signed_form(
        {
            "spu_guid": str(spu_guid),
            "module_id": str(module_id),
        }
    )


def build_app_info_form(app_guid: str, terminal_id: str = "4") -> dict[str, str]:
    return build_signed_form(
        {
            "app_guid": str(app_guid),
            "terminal_id": str(terminal_id),
        }
    )


def build_other_token_form(
    token_code: str,
    *,
    app_guid: str,
    company_guid: str,
    idaas_id: str,
) -> dict[str, str]:
    return build_signed_form(
        {
            "token_code": token_code,
            "app_guid": str(app_guid),
            "company_guid": str(company_guid),
            "idaas_id": str(idaas_id),
        }
    )


def parse_iaaa_oauth_url(oauth_url: str) -> tuple[str, str]:
    parsed = urlparse(oauth_url)
    query = parse_qs(parsed.query)
    redir_url = query.get("redirectUrl", [None])[0]
    app_id = query.get("appID", [None])[0]
    if not redir_url or not app_id:
        raise ValueError("IAAA oauth URL must contain redirectUrl and appID query params")
    return redir_url, app_id


def build_callback_url(app_guid: str, *, path: str | None = None, extra: dict[str, str] | None = None) -> str:
    cb = uuid.uuid4().hex[:16]
    path = path or f"/{app_guid}/home"
    query = {
        "logintype": "sf",
        "cb": cb,
        **(extra or {}),
    }
    qs = "&".join(f"{key}={requests.utils.quote(str(value), safe='')}" for key, value in query.items())
    return f"https://byyxt.pupedu.cn{path}?{qs}"


def build_beida_entry_url(app_info: dict, *, callback_url: str | None = None, a_uri: dict | None = None) -> str:
    callback_url = callback_url or build_callback_url(str(app_info["app_guid"]), extra={"f": "bd", "r": "2"})
    a_uri = a_uri or {}
    base = app_info["idp"].get("domain") or "https://idp.readoor.cn/"
    base = base.rstrip("/")
    return (
        f"{base}/api/3.0/idp/v1/ag/bd?"
        f"&terminal_id=4"
        f"&mode=20"
        f"&app_guid={requests.utils.quote(str(app_info['app_guid']), safe='')}"
        f"&appid={requests.utils.quote(str(app_info['idp']['idaas_app_id']), safe='')}"
        f"&company_guid={requests.utils.quote(str(app_info['company_guid']), safe='')}"
        f"&callback={requests.utils.quote(callback_url, safe='')}"
        f"&a_uri={requests.utils.quote(json.dumps(a_uri, separators=(',', ':')), safe='')}"
    )


def fetch_dynamic_iaaa_oauth_url(app_info: dict) -> str:
    a_uri = {}
    for mode in app_info.get("idp", {}).get("config", {}).get("mode_config", []) or []:
        if str(mode.get("mode_id")) == "20":
            rel = (((mode.get("ext_config") or {}).get("rel")) or {})
            if rel.get("enterprise_id") not in (None, ""):
                a_uri["enterprise_id"] = rel["enterprise_id"]
            if rel.get("org_id") not in (None, ""):
                a_uri["org_id"] = rel["org_id"]
            break
    entry_url = build_beida_entry_url(app_info, a_uri=a_uri)
    response = requests.get(entry_url, timeout=30, allow_redirects=False)
    location = response.headers.get("Location") or response.headers.get("location")
    if not location or "iaaa.pku.edu.cn/iaaa/oauth.jsp" not in location:
        raise RuntimeError(
            "Could not fetch dynamic IAAA oauth URL from beida entrypoint. "
            f"status={response.status_code} location={location!r}"
        )
    return location


def fetch_app_info(app_guid: str, terminal_id: str = "4") -> dict:
    response = requests.post(APP_INFO_ENDPOINT, data=build_app_info_form(app_guid, terminal_id), timeout=30)
    response.raise_for_status()
    payload = response.json()
    if str(payload.get("status")) != "1":
        raise RuntimeError(f"app info request failed: {payload}")
    app_info = payload["data"]
    # Frontend patches app_guid back onto the app-info payload because this
    # field is not always present in the API response body.
    app_info["app_guid"] = str(app_guid)
    return app_info


def login_iaaa_for_token_code(
    username: str,
    password: str,
    *,
    oauth_url: str | None = None,
    app_info: dict | None = None,
) -> tuple[str, dict]:
    if oauth_url is None:
        if app_info is None:
            raise ValueError("Need either oauth_url or app_info to start IAAA login")
        oauth_url = fetch_dynamic_iaaa_oauth_url(app_info)
    redir_url, app_id = parse_iaaa_oauth_url(oauth_url)
    session = requests.Session()
    response = session.post(
        IAAA_LOGIN_ENDPOINT,
        data={
            "appid": app_id,
            "userName": username,
            "password": password,
            "randCode": "",
            "smsCode": "",
            "otpCode": "",
            "redirUrl": redir_url,
        },
        timeout=30,
    )
    response.raise_for_status()
    login_payload = response.json()
    if not login_payload.get("success"):
        raise RuntimeError(f"IAAA login failed: {login_payload}")
    oauth_token = login_payload.get("token")
    if not oauth_token:
        raise RuntimeError(f"IAAA login did not return token: {login_payload}")

    callback_response = session.get(
        redir_url,
        params={
            "_rand": str(uuid.uuid4().int / 10**38),
            "token": oauth_token,
        },
        timeout=30,
        allow_redirects=False,
    )
    location = callback_response.headers.get("Location") or callback_response.headers.get("location") or callback_response.url
    token_code = parse_qs(urlparse(location).query).get("token_code", [None])[0]
    if not token_code:
        raise RuntimeError(
            "Could not extract token_code from IAAA callback. "
            f"status={callback_response.status_code} location={location!r}"
        )
    return token_code, {"oauth_token": oauth_token, "oauth_url": oauth_url, "location": location}


def exchange_token_code(token_code: str, app_info: dict) -> dict:
    endpoint = app_info["domain"]["idp"].rstrip("/") + "/api/3.0/idp/v1/s/ag/token"
    form_payload = build_other_token_form(
        token_code,
        app_guid=str(app_info["app_guid"]),
        company_guid=str(app_info["company_guid"]),
        idaas_id=str(app_info["idp"]["idaas_id"]),
    )
    response = requests.post(endpoint, data=form_payload, timeout=30)
    response.raise_for_status()
    payload = response.json()
    if str(payload.get("status")) != "1":
        raise RuntimeError(f"token_code exchange failed: {payload}")
    return payload


def dynamic_done_field(timestamp_ms: int) -> str:
    return base64.b64encode(str(timestamp_ms).encode("utf-8")).decode("ascii")[2:8]


def build_payload(
    *,
    spu_guid: str,
    task_guid: str,
    complete: bool,
    session_id: str | None,
    sequence_id: int | None,
    study_time: int | None,
    position: float | None,
) -> dict:
    now_ms = int(time.time() * 1000)
    now_s = now_ms // 1000
    lesson_study_time = study_time if study_time is not None else DEFAULT_STUDY_TIME
    lesson_position = position if position is not None else DEFAULT_MEDIA_DURATION
    lesson_session_id = session_id or str(uuid.uuid4())
    lesson_sequence_id = sequence_id if sequence_id is not None else DEFAULT_SEQUENCE_ID

    payload = {
        "base_data": {
            "app_id": 0,
            "company_id": 0,
            "time_stamp": now_ms,
            "class_id": COURSE_CLASS_ID,
            "train_id": COURSE_TRAIN_ID,
            "project_id": COURSE_PROJECT_ID,
            "platform_code": DEFAULT_PLATFORM_CODE,
            "item_id": str(spu_guid),
            "spu_type": COURSE_SPU_TYPE,
        },
        "lesson_data": [
            {
                "media_theory_length": DEFAULT_MEDIA_DURATION,
                "max_position": lesson_position,
                "position": lesson_position,
                "study_time": lesson_study_time,
                "end_time": now_s,
                "start_time": now_s - int(lesson_study_time),
                "session_id": lesson_session_id,
                "sequence_id": lesson_sequence_id,
                "section_guid": DEFAULT_SECTION_GUID,
                "courseware_type": DEFAULT_COURSEWARE_TYPE,
                "section_type": DEFAULT_SECTION_TYPE,
                "task_guid": str(task_guid),
            }
        ],
    }
    lesson = payload["lesson_data"][0]

    done_key = dynamic_done_field(now_ms)
    lesson[done_key] = "1" if complete else 0

    if complete:
        lesson["position"] = lesson["media_theory_length"]
        lesson["max_position"] = lesson["media_theory_length"]

    return payload


def apply_section_to_payload(
    payload: dict,
    section: dict,
    *,
    spu_guid: str | None,
    task_guid: str | None,
) -> dict:
    lesson = payload["lesson_data"][0]
    if spu_guid:
        payload["base_data"]["item_id"] = str(spu_guid)
    lesson["section_guid"] = str(section["section_guid"])
    lesson["courseware_type"] = int(section["courseware_type"])
    lesson["section_type"] = int(section["section_type"])
    if "file_duration" in section:
        lesson["media_theory_length"] = float(section["file_duration"])
        lesson["position"] = float(section["file_duration"])
        lesson["max_position"] = float(section["file_duration"])
    if task_guid:
        lesson["task_guid"] = str(task_guid)
    return payload


def apply_app_info_to_payload(payload: dict, app_info: dict) -> dict:
    payload["base_data"]["app_id"] = int(app_info["app_id"])
    payload["base_data"]["company_id"] = int(app_info["company_id"])
    return payload


def encrypt_payload(payload: dict) -> dict[str, str]:
    plain = json.dumps(payload, separators=(",", ":"), ensure_ascii=False).encode("utf-8")
    iv = os.urandom(16)
    cipher = AES.new(KEY, AES.MODE_CBC, iv=iv)
    encrypted = cipher.encrypt(pad(plain, AES.block_size))
    return {
        "data": base64.b64encode(encrypted).decode("ascii"),
        "jfug": base64.b64encode(iv).decode("ascii"),
    }


def build_headers(token: str) -> dict[str, str]:
    headers = {
        "Authorization": f"Bearer {token}",
        "Content-Type": "application/x-www-form-urlencoded",
        "Origin": "https://byyxt.pupedu.cn",
        "Referer": REFERER,
        "User-Agent": (
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) "
            "AppleWebKit/537.36 (KHTML, like Gecko) "
            "Chrome/137.0.0.0 Safari/537.36"
        ),
        "X-Requested-With": "XMLHttpRequest",
    }
    return headers


def send_probe(token: str, form_payload: dict[str, str]) -> requests.Response:
    headers = build_headers(token)
    return requests.post(ENDPOINT, headers=headers, data=form_payload, timeout=30)


def fetch_sections(token: str, spu_guid: str, module_id: str) -> requests.Response:
    headers = build_headers(token)
    form_payload = build_sections_form(spu_guid, module_id)
    return requests.post(SECTIONS_ENDPOINT, headers=headers, data=form_payload, timeout=30)


def load_sections_file(path: str) -> dict:
    with open(path, "r", encoding="utf-8") as f:
        return json.load(f)


def format_section_summary(section: dict) -> dict:
    return {
        "section_guid": section.get("section_guid"),
        "section_name": section.get("section_name"),
        "courseware_type": section.get("courseware_type"),
        "section_type": section.get("section_type"),
        "file_duration": section.get("file_duration"),
        "id": section.get("id"),
        "pid": section.get("pid"),
    }


def list_playable_sections(sections_payload: dict) -> list[dict]:
    sections = sections_payload.get("data", {}).get("sections", [])
    return [
        section
        for section in sections
        if section.get("section_type") == 403 and section.get("courseware_type")
    ]


def parse_multi_value(raw: str) -> list[str]:
    return [item.strip() for item in raw.split(",") if item.strip()]


def resolve_sections(sections_payload: dict, selectors: list[str] | None = None) -> list[dict]:
    playable_sections = list_playable_sections(sections_payload)
    if not playable_sections:
        raise ValueError("No playable sections found in payload")

    if not selectors:
        return [playable_sections[0]]

    chosen_sections: list[dict] = []
    for selector in selectors:
        section = None
        if selector.isdigit():
            index = int(selector)
            if 1 <= index <= len(playable_sections):
                section = playable_sections[index - 1]
        if section is None:
            for candidate in playable_sections:
                if str(candidate.get("section_guid")) == selector:
                    section = candidate
                    break
        if section is None:
            raise ValueError(f"Invalid section selector: {selector}")
        if all(str(existing.get("section_guid")) != str(section.get("section_guid")) for existing in chosen_sections):
            chosen_sections.append(section)
    return chosen_sections


def prompt_for_sections(sections_payload: dict) -> list[dict]:
    playable_sections = list_playable_sections(sections_payload)
    if not playable_sections:
        raise ValueError("No playable sections found in payload")

    print("\n=== selectable sections ===")
    for index, section in enumerate(playable_sections, start=1):
        summary = format_section_summary(section)
        print(
            f"[{index}] {summary['section_name']} | "
            f"guid={summary['section_guid']} | "
            f"type={summary['courseware_type']} | "
            f"duration={summary['file_duration']}"
        )

    while True:
        raw = input("Choose section number(s) or section_guid(s), comma-separated: ").strip()
        if not raw:
            print("Please enter a value.")
            continue
        try:
            return resolve_sections(sections_payload, parse_multi_value(raw))
        except ValueError:
            print("Invalid selection, try again.")


def choose_sections(sections_payload: dict, section_guid: str | None) -> list[dict]:
    sections = sections_payload.get("data", {}).get("sections", [])
    if not sections:
        raise ValueError("No sections found in payload")
    if section_guid:
        return resolve_sections(sections_payload, parse_multi_value(section_guid))
    return resolve_sections(sections_payload, None)


def main() -> None:
    parser = argparse.ArgumentParser(description="Probe Readoor pStatIf, with optional PKU IAAA login and token_code exchange.")
    parser.add_argument("--token", default=os.environ.get("READOOR_TOKEN"), help="Bearer token. If omitted, the script can login with --username/--password.")
    parser.add_argument("--username", default=os.environ.get("READOOR_USERNAME"), help="PKU username for IAAA login.")
    parser.add_argument("--password", default=os.environ.get("READOOR_PASSWORD"), help="PKU password for IAAA login.")
    parser.add_argument("--token-code", help="Skip IAAA login and exchange this token_code directly.")
    parser.add_argument("--iaaa-oauth-url", help="Optional full IAAA oauth.jsp URL. Usually not needed; the script can fetch a fresh one dynamically.")
    parser.add_argument("--app-guid", default="550278742975483904", help="App guid used for app info and token exchange.")
    parser.add_argument("--terminal-id", default="4", help="Terminal id used in app info/signature requests. PC is 4.")
    parser.add_argument("--login-only", action="store_true", help="Only perform login / token exchange and print the Bearer token.")
    parser.add_argument("--spu-guid", default=COURSE_SPU_GUID, help="spu_guid / item_id.")
    parser.add_argument("--module-id", help="module_id for the sections API.")
    parser.add_argument("--section-guid", help="Pick a section by section_guid from API/file data.")
    parser.add_argument("--task-guid", default=COURSE_TASK_GUID, help="Optional task_guid to attach to pStatIf.")
    parser.add_argument("--sections-file", help="Use a saved sections response JSON instead of calling the sections API.")
    parser.add_argument("--list-sections", action="store_true", help="Fetch or load sections and print them.")
    parser.add_argument("--choose-section", action="store_true", help="Interactively choose a playable section after loading sections.")
    parser.add_argument("--session-id", help="Reuse an existing session_id. Defaults to a new uuid4.")
    parser.add_argument("--sequence-id", type=int, help="Override sequence_id.")
    parser.add_argument("--study-time", type=int, default=DEFAULT_STUDY_TIME, help="study_time in seconds.")
    parser.add_argument("--position", type=float, help="Override position/max_position. Ignored when --incomplete is not set.")
    parser.add_argument("--incomplete", action="store_true", help="Send an unfinished payload instead of a completed one.")
    parser.add_argument("--dump-only", action="store_true", help="Only print JSON and encrypted form, do not send.")
    args = parser.parse_args()

    app_info = fetch_app_info(args.app_guid, args.terminal_id)
    token = args.token
    token_code = args.token_code

    if not token and not token_code and args.username and args.password:
        token_code, login_meta = login_iaaa_for_token_code(
            args.username,
            args.password,
            oauth_url=args.iaaa_oauth_url,
            app_info=app_info,
        )
        print("=== IAAA callback ===")
        print(json.dumps(login_meta, indent=2, ensure_ascii=False))

    if not token and token_code:
        token_payload = exchange_token_code(token_code, app_info)
        print("=== token exchange ===")
        print(json.dumps(token_payload, indent=2, ensure_ascii=False))
        token = token_payload["data"]["token"]["token"]

    if args.login_only:
        if not token:
            raise SystemExit("Login did not produce a Bearer token.")
        print("\n=== bearer token ===")
        print(token)
        return

    sections_payload = None
    if args.sections_file:
        sections_payload = load_sections_file(args.sections_file)
    elif args.list_sections or args.module_id or args.section_guid:
        if not token:
            raise SystemExit("Missing token for sections API. Pass --token or login with --username/--password.")
        if not args.module_id:
            raise SystemExit("module_id is required when fetching sections from the API.")
        sections_response = fetch_sections(token, args.spu_guid, args.module_id)
        print("=== sections HTTP response ===")
        print(f"status_code={sections_response.status_code}")
        print(sections_response.text)
        sections_payload = sections_response.json()

    if args.list_sections and sections_payload is not None:
        print("\n=== sections summary ===")
        for section in sections_payload.get("data", {}).get("sections", []):
            print(json.dumps(format_section_summary(section), ensure_ascii=False))
        if args.dump_only:
            return

    selected_sections: list[dict | None]
    if sections_payload is not None:
        if args.choose_section and not args.section_guid:
            selected_sections = prompt_for_sections(sections_payload)
        else:
            selected_sections = choose_sections(sections_payload, args.section_guid)
    else:
        selected_sections = [None]

    for index, section in enumerate(selected_sections, start=1):
        payload = build_payload(
            spu_guid=args.spu_guid,
            task_guid=args.task_guid,
            complete=not args.incomplete,
            session_id=args.session_id,
            sequence_id=args.sequence_id,
            study_time=args.study_time,
            position=args.position,
        )
        payload = apply_app_info_to_payload(payload, app_info)
        if section is not None:
            print(f"\n=== chosen section {index}/{len(selected_sections)} ===")
            print(json.dumps(format_section_summary(section), indent=2, ensure_ascii=False))
            payload = apply_section_to_payload(payload, section, spu_guid=args.spu_guid, task_guid=args.task_guid)

        encrypted = encrypt_payload(payload)

        print(f"\n=== JSON payload {index}/{len(selected_sections)} ===")
        print(json.dumps(payload, indent=2, ensure_ascii=False))
        print(f"\n=== Form payload {index}/{len(selected_sections)} ===")
        print(json.dumps(encrypted, indent=2, ensure_ascii=False))

        if args.dump_only:
            continue

        if not token:
            raise SystemExit("Missing token. Pass --token or login with --username/--password.")

        response = send_probe(token, encrypted)
        print(f"\n=== HTTP response {index}/{len(selected_sections)} ===")
        print(f"status_code={response.status_code}")
        print(response.text)


if __name__ == "__main__":
    main()
