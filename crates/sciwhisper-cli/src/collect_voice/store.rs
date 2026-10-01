//! A session is an immutable plan and independently committed takes.
//! Rename publishes a complete take; an OS file lock serialises all writers.
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use sciwhisper_asr::corpus::describe_wav;
use sciwhisper_eval::schema::{AudioSource, Consent, Dataset, Provenance, Record, Split};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const CONSENT: &str = "Согласие voice-local-v1\nЯ разрешаю sci-witch сохранять короткие записи моего голоса и подтверждённый мною текст для локальной проверки качества. Голос может позволить узнать меня: код диктора не делает его анонимным. Записи, сведения о микрофоне и обстановке остаются в выбранной папке. Экспорт создаёт ещё одну локальную копию, без отправки и разрешения на публикацию. Для передачи другим людям нужно отдельное согласие. Команда revoke удаляет записи этой сессии; ранее экспортированные или переданные копии нужно удалить отдельно. Автоматическое обучение не выполняется.\n";
const LIMIT: u64 = 8 * 1024 * 1024;
pub const MAX_TASKS: usize = 1000;

pub type Result<T> = std::result::Result<T, String>;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    pub schema_version: u32,
    pub session_id: String,
    pub speaker_id: String,
    pub split: Split,
    pub consent: Consent,
    pub consent_sha256: String,
    pub source_sha256: String,
    pub environment: String,
    pub tasks: Vec<Record>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Take {
    pub schema_version: u32,
    pub task_index: usize,
    pub record: Record,
    pub has_speech: bool,
    pub os: String,
}

pub struct Session {
    pub root: PathBuf,
    pub plan: Plan,
    _lock: File,
}

pub fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub fn read_limited(path: &Path) -> Result<Vec<u8>> {
    regular(path)?;
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(err)?
        .take(LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(err)?;
    if bytes.len() as u64 > LIMIT {
        return Err("Файл превышает предел 8 МиБ".into());
    }
    Ok(bytes)
}

pub fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

pub fn valid_date(s: &str) -> bool {
    if s.len() != 10
        || !s.bytes().enumerate().all(|(i, b)| {
            if i == 4 || i == 7 {
                b == b'-'
            } else {
                b.is_ascii_digit()
            }
        })
    {
        return false;
    }
    let (Ok(y), Ok(m), Ok(d)) = (
        s[..4].parse::<u32>(),
        s[5..7].parse::<u32>(),
        s[8..].parse::<u32>(),
    ) else {
        return false;
    };
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let days = match m {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        _ => 0,
    };
    (2000..=9999).contains(&y) && d > 0 && d <= days
}

fn validate_plan(plan: &Plan) -> Result<()> {
    if plan.schema_version != 1
        || plan.session_id.len() != 32
        || !plan.session_id.bytes().all(|b| b.is_ascii_hexdigit())
        || !plan.consent.granted
        || plan.consent.statement_id != "voice-local-v1"
        || plan.consent_sha256 != digest(CONSENT.as_bytes())
        || !valid_date(&plan.consent.date)
    {
        return Err("Неизвестная версия сессии или неподтверждённое согласие".into());
    }
    let id = &plan.speaker_id;
    if !id.starts_with("spk-")
        || !(8..=40).contains(&id.len())
        || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return Err("Код диктора: spk- и 4–36 латинских букв/цифр; используйте один код для одного человека".into());
    }
    if plan.environment.trim().is_empty()
        || plan.environment.len() > 200
        || plan.environment.contains(['\n', '\r'])
    {
        return Err("Обстановка должна быть краткой, без личных данных".into());
    }
    if plan.source_sha256.len() != 64
        || !plan.source_sha256.bytes().all(|b| b.is_ascii_hexdigit())
        || plan.tasks.is_empty()
        || plan.tasks.len() > MAX_TASKS
    {
        return Err("Неверный план заданий".into());
    }
    let jsonl = plan
        .tasks
        .iter()
        .map(|r| serde_json::to_string(r).map_err(err))
        .collect::<Result<Vec<_>>>()?
        .join("\n");
    Dataset::parse_jsonl(&jsonl).map_err(err)?;
    for task in &plan.tasks {
        if task.split != plan.split
            || !matches!(
                task.provenance,
                Provenance::HandcraftedText | Provenance::SyntheticText
            )
            || task.human_transcript.len() > 8192
        {
            return Err("Задания должны быть текстовыми и принадлежать одному split".into());
        }
    }
    Ok(())
}

pub fn plan(
    source: &[u8],
    split: Split,
    speaker_id: String,
    date: String,
    environment: String,
) -> Result<Plan> {
    let text = std::str::from_utf8(source).map_err(err)?;
    if source.len() as u64 > LIMIT {
        return Err("План слишком велик".into());
    }
    let dataset = Dataset::parse_jsonl(text).map_err(err)?;
    let mut families = BTreeMap::new();
    for record in &dataset.records {
        if families
            .insert(&record.family_id, record.split)
            .is_some_and(|previous| previous != record.split)
        {
            return Err("Семейство заданий пересекает split".into());
        }
    }
    let plan = Plan {
        schema_version: 1,
        // An opaque uniqueness token, not an identity or authentication key.
        session_id: digest(
            format!(
                "{}:{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_err(err)?
                    .as_nanos()
            )
            .as_bytes(),
        )[..32]
            .into(),
        speaker_id,
        split,
        consent: Consent {
            granted: true,
            statement_id: "voice-local-v1".into(),
            date,
        },
        consent_sha256: digest(CONSENT.as_bytes()),
        source_sha256: digest(source),
        environment,
        tasks: dataset
            .records
            .into_iter()
            .filter(|r| r.split == split)
            .collect(),
    };
    validate_plan(&plan)?;
    Ok(plan)
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(err)?;
    f.write_all(bytes).map_err(err)?;
    f.sync_all().map_err(err)
}

fn regular(path: &Path) -> Result<()> {
    if !fs::symlink_metadata(path)
        .map_err(err)?
        .file_type()
        .is_file()
    {
        return Err("Ожидался обычный файл без символической ссылки".into());
    }
    Ok(())
}

fn directory(path: &Path) -> Result<()> {
    if !fs::symlink_metadata(path)
        .map_err(err)?
        .file_type()
        .is_dir()
    {
        return Err("Ожидался каталог без символической ссылки".into());
    }
    Ok(())
}

fn check_pcm(path: &Path) -> Result<()> {
    let reader = hound::WavReader::open(path).map_err(err)?;
    let spec = reader.spec();
    if spec.sample_format != hound::SampleFormat::Int
        || spec.bits_per_sample != 16
        || spec.channels != 1
        || spec.sample_rate != 16000
        || !(1600..=960000).contains(&reader.duration())
    {
        return Err("Нужен PCM16 моно WAV 16 кГц продолжительностью 0,1–60 секунд".into());
    }
    Ok(())
}

impl Session {
    /// Only called after the interactive consent confirmation.
    pub fn create(root: &Path, plan: Plan) -> Result<Self> {
        validate_plan(&plan)?;
        fs::create_dir(root).map_err(|e| format!("Для сессии нужна новая папка: {e}"))?;
        write_new(
            &root.join("session.json"),
            &serde_json::to_vec_pretty(&plan).map_err(err)?,
        )?;
        write_new(&root.join("consent.txt"), CONSENT.as_bytes())?;
        fs::create_dir(root.join("takes")).map_err(err)?;
        Self::open(root)
    }

    pub fn open(root: &Path) -> Result<Self> {
        directory(root)?;
        let lock_path = root.join("session.lock");
        if lock_path.exists() || fs::symlink_metadata(&lock_path).is_ok() {
            regular(&lock_path)?;
        }
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock_path)
            .map_err(err)?;
        lock.try_lock()
            .map_err(|_| "Сессия уже открыта в другом процессе".to_string())?;
        let plan: Plan =
            serde_json::from_slice(&read_limited(&root.join("session.json"))?).map_err(err)?;
        validate_plan(&plan)?;
        if read_limited(&root.join("consent.txt"))? != CONSENT.as_bytes() {
            return Err("Текст согласия изменён".into());
        }
        directory(&root.join("takes"))?;
        Ok(Self {
            root: root.to_owned(),
            plan,
            _lock: lock,
        })
    }

    pub fn active(&self) -> Result<()> {
        if self.root.join("REVOKED").try_exists().map_err(err)? {
            return Err("Согласие отозвано: запись и экспорт запрещены".into());
        }
        Ok(())
    }

    pub fn takes(&self) -> Result<BTreeMap<usize, Take>> {
        self.active()?;
        let mut takes = BTreeMap::new();
        for item in fs::read_dir(self.root.join("takes")).map_err(err)? {
            let item = item.map_err(err)?;
            let name = item.file_name().to_string_lossy().to_string();
            // An interrupted transaction is not an accepted recording.
            if name.starts_with(".pending-") {
                continue;
            }
            directory(&item.path())?;
            let take: Take = serde_json::from_slice(&read_limited(&item.path().join("take.json"))?)
                .map_err(err)?;
            let idx = take.task_index;
            if take.schema_version != 1
                || name != format!("{idx:04}")
                || idx >= self.plan.tasks.len()
                || takes.contains_key(&idx)
            {
                return Err("Повреждён индекс записи".into());
            }
            let mut expected = self.plan.tasks[idx].clone();
            expected.dataset_schema_version = 2;
            expected.id = format!(
                "{}-{}-{}",
                expected.id, self.plan.speaker_id, self.plan.session_id
            );
            expected.provenance = Provenance::RealAudio;
            expected.speaker_id = Some(self.plan.speaker_id.clone());
            expected.audio = take.record.audio.clone();
            if expected != take.record {
                return Err("Эталон или текст сохранённой записи изменён".into());
            }
            let audio = take.record.audio.as_ref().ok_or("В записи нет аудио")?;
            if audio.file != format!("audio/{idx:04}.wav")
                || audio.consent != self.plan.consent
                || audio.environment.as_deref() != Some(&self.plan.environment)
            {
                return Err("Метаданные записи не совпадают с сессией".into());
            }
            let path = item.path().join("audio.wav");
            regular(&path)?;
            if fs::metadata(&path).map_err(err)?.len() > 2_000_000 {
                return Err("Аудио превышает предел 60 секунд".into());
            }
            let facts = describe_wav(&path).map_err(err)?;
            check_pcm(&path)?;
            if audio.sha256 != facts.sha256
                || (audio.duration_secs - facts.duration_secs).abs() > 1e-9
                || audio.sample_rate_hz != facts.sample_rate_hz
                || audio.channels != facts.channels
                || !match (audio.snr_db, facts.snr_db) {
                    (Some(a), Some(b)) => (a - b).abs() < 1e-9,
                    (None, None) => true,
                    _ => false,
                }
                || take.has_speech != facts.has_speech
            {
                return Err("Аудио изменено после подтверждения".into());
            }
            Dataset::parse_jsonl(&serde_json::to_string(&take.record).map_err(err)?)
                .map_err(err)?;
            takes.insert(idx, take);
        }
        Ok(takes)
    }

    pub fn accept(&self, idx: usize, wav: &Path, transcript: &str, microphone: &str) -> Result<()> {
        self.active()?;
        let mut record = self
            .plan
            .tasks
            .get(idx)
            .ok_or("Нет такого задания")?
            .clone();
        if transcript.trim() != record.human_transcript {
            return Err("Произнесённый текст отличается от задания. Перезапишите: эталон относится только к исходной фразе".into());
        }
        if microphone.trim().is_empty() || microphone.len() > 200 {
            return Err("Неверное описание микрофона".into());
        }
        regular(wav)?;
        if fs::metadata(wav).map_err(err)?.len() > 2_000_000 {
            return Err("Аудио слишком велико".into());
        }
        let stage = tempfile::Builder::new()
            .prefix(".pending-")
            .tempdir_in(self.root.join("takes"))
            .map_err(err)?;
        fs::copy(wav, stage.path().join("audio.wav")).map_err(err)?;
        check_pcm(&stage.path().join("audio.wav"))?;
        let facts = describe_wav(&stage.path().join("audio.wav")).map_err(err)?;
        if facts.sample_rate_hz != 16000
            || facts.channels != 1
            || !(0.1..=60.0).contains(&facts.duration_secs)
        {
            return Err("Нужен моно WAV 16 кГц продолжительностью 0,1–60 секунд".into());
        }
        record.dataset_schema_version = 2;
        record.id = format!(
            "{}-{}-{}",
            record.id, self.plan.speaker_id, self.plan.session_id
        );
        record.provenance = Provenance::RealAudio;
        record.speaker_id = Some(self.plan.speaker_id.clone());
        record.audio = Some(AudioSource {
            file: format!("audio/{idx:04}.wav"),
            sha256: facts.sha256,
            duration_secs: facts.duration_secs,
            sample_rate_hz: facts.sample_rate_hz,
            channels: facts.channels,
            microphone: Some(microphone.into()),
            environment: Some(self.plan.environment.clone()),
            snr_db: facts.snr_db,
            consent: self.plan.consent.clone(),
        });
        Dataset::parse_jsonl(&serde_json::to_string(&record).map_err(err)?).map_err(err)?;
        let take = Take {
            schema_version: 1,
            task_index: idx,
            record,
            has_speech: facts.has_speech,
            os: std::env::consts::OS.into(),
        };
        write_new(
            &stage.path().join("take.json"),
            &serde_json::to_vec_pretty(&take).map_err(err)?,
        )?;
        OpenOptions::new()
            .write(true)
            .open(stage.path().join("audio.wav"))
            .map_err(err)?
            .sync_all()
            .map_err(err)?;
        let dest = self.root.join("takes").join(format!("{idx:04}"));
        if dest.try_exists().map_err(err)? {
            return Err("Это задание уже сохранено".into());
        }
        fs::rename(stage.path(), dest).map_err(err)?;
        Ok(())
    }

    pub fn export(&self, destination: &Path) -> Result<usize> {
        let takes = self.takes()?;
        if takes.is_empty() {
            return Err("Нет подтверждённых записей для экспорта".into());
        }
        if destination.try_exists().map_err(err)? {
            return Err("Папка экспорта уже существует; выберите новую".into());
        }
        let parent = destination
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let stage = tempfile::Builder::new()
            .prefix(".voice-export-")
            .tempdir_in(parent)
            .map_err(err)?;
        fs::create_dir(stage.path().join("audio")).map_err(err)?;
        let mut jsonl = String::new();
        let mut hashes = BTreeMap::<String, String>::new();
        for (idx, take) in &takes {
            let audio = take.record.audio.as_ref().ok_or("Нет аудио")?;
            let to = stage.path().join(&audio.file);
            fs::copy(self.root.join(format!("takes/{idx:04}/audio.wav")), &to).map_err(err)?;
            let hash = sciwhisper_asr::model::sha256_file(&to).map_err(err)?;
            if hash != audio.sha256 {
                return Err("Аудио изменилось во время экспорта".into());
            }
            hashes.insert(audio.file.clone(), hash);
            jsonl.push_str(&serde_json::to_string(&take.record).map_err(err)?);
            jsonl.push('\n');
        }
        Dataset::parse_jsonl(&jsonl).map_err(err)?;
        for (name, bytes) in [
            ("dataset.jsonl", jsonl.as_bytes()),
            ("consent.txt", CONSENT.as_bytes()),
        ] {
            write_new(&stage.path().join(name), bytes)?;
            hashes.insert(name.into(), digest(bytes));
        }
        let manifest = serde_json::json!({ "voice_pack_schema_version": 1, "collector_version": env!("CARGO_PKG_VERSION"),
            "source_sha256": self.plan.source_sha256, "speaker_id": self.plan.speaker_id, "session_id": self.plan.session_id,
            "split": self.plan.split, "scope": "local_only", "records": takes.len(),
            "planned_records": self.plan.tasks.len(), "recording_os": takes.values().map(|t| t.os.as_str()).collect::<BTreeSet<_>>(),
            "audio_processing": "mono_pcm16_16000_no_vad_trim", "files": hashes });
        write_new(
            &stage.path().join("pack.json"),
            &serde_json::to_vec_pretty(&manifest).map_err(err)?,
        )?;
        fs::rename(stage.path(), destination).map_err(err)?;
        Ok(takes.len())
    }

    /// Mark revoked before any deletion. If interrupted, rerunning finishes
    /// the removal; recordings cannot be exported in the intervening state.
    pub fn revoke(&self) -> Result<()> {
        let marker = self.root.join("REVOKED");
        if !marker.try_exists().map_err(err)? {
            write_new(&marker, b"Local consent withdrawn\n")?;
        }
        for entry in fs::read_dir(self.root.join("takes")).map_err(err)? {
            let entry = entry.map_err(err)?;
            let name = entry.file_name().to_string_lossy().to_string();
            if !(name.starts_with(".pending-")
                || (name.len() == 4 && name.bytes().all(|b| b.is_ascii_digit())))
            {
                return Err("Посторонний файл в takes; удаление остановлено".into());
            }
            directory(&entry.path())?;
            let allowed: BTreeSet<&str> = ["take.json", "audio.wav"].into_iter().collect();
            for file in fs::read_dir(entry.path()).map_err(err)? {
                let file = file.map_err(err)?;
                if !allowed.contains(file.file_name().to_string_lossy().as_ref()) {
                    return Err("Посторонний файл в записи; удаление остановлено".into());
                }
                regular(&file.path())?;
                fs::remove_file(file.path()).map_err(err)?;
            }
            fs::remove_dir(entry.path()).map_err(err)?;
        }
        Ok(())
    }
}
