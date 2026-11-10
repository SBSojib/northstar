use std::{
    io::{self, BufRead, BufReader, Write},
    path::Path,
};

const MAX_ANSWER: usize = 1024 * 1024;

pub fn main() -> ! {
    let code = match respond() {
        Ok(()) => 0,
        Err(error) => {
            let _ = writeln!(io::stderr(), "northstar askpass: {error}");
            1
        }
    };
    std::process::exit(code);
}

fn respond() -> io::Result<()> {
    let dir = std::env::var("NORTHSTAR_ASKPASS_DIR")
        .map_err(|_| io::Error::new(io::ErrorKind::NotFound, "missing prompt directory"))?;
    let prompt = std::env::args()
        .nth(1)
        .unwrap_or_default()
        .replace(['\n', '\r'], " ");
    let dir = Path::new(&dir);
    {
        let mut request = std::fs::OpenOptions::new()
            .write(true)
            .open(dir.join("request"))?;
        writeln!(request, "{prompt}")?;
        request.flush()?;
    }
    let response = std::fs::File::open(dir.join("response"))?;
    let answer = read_answer(&mut BufReader::new(response))?;
    let mut stdout = io::stdout().lock();
    stdout.write_all(&answer)?;
    stdout.write_all(b"\n")?;
    stdout.flush()?;
    Ok(())
}

pub(crate) fn encode_answer(answer: &str) -> Vec<u8> {
    let bytes = answer.as_bytes();
    let mut payload = format!("{}\n", bytes.len()).into_bytes();
    payload.extend_from_slice(bytes);
    payload
}

pub(crate) fn read_answer(reader: &mut impl BufRead) -> io::Result<Vec<u8>> {
    let mut line = String::new();
    if reader.read_line(&mut line)? == 0 {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "missing prompt answer",
        ));
    }
    let len: usize = line.trim().parse().map_err(|_| {
        io::Error::new(io::ErrorKind::InvalidData, "invalid prompt answer length")
    })?;
    if len > MAX_ANSWER {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "prompt answer is too large",
        ));
    }
    let mut buffer = vec![0_u8; len];
    reader.read_exact(&mut buffer)?;
    Ok(buffer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn answer_round_trip_keeps_internal_newlines() {
        let answer = "correct\nhorse";
        let mut cursor = Cursor::new(encode_answer(answer));
        let decoded = read_answer(&mut cursor).expect("decode");
        assert_eq!(decoded, answer.as_bytes());
    }
}
