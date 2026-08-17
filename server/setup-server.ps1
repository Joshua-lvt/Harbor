# Harbor relay bootstrap — cria o venv, instala dependências e roda os testes.
#
# Execute a partir da pasta server/:
#   cd server
#   ./setup-server.ps1
#
# Para rodar só os testes depois: .\.venv\Scripts\python.exe -m pytest -q

$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path

Push-Location $here
try {
    if (-not (Test-Path '.venv')) {
        Write-Host "Criando ambiente virtual (.venv) ..."
        python -m venv .venv
    }

    $py = '.venv\Scripts\python.exe'
    & $py -m pip install --quiet --upgrade pip
    & $py -m pip install --quiet -e ".[dev]"

    Write-Host "`nRodando pytest ..." -ForegroundColor Cyan
    & $py -m pytest -q

    Write-Host "`nPara iniciar o relay em desenvolvimento:" -ForegroundColor Green
    Write-Host "  .venv\Scripts\python.exe -m uvicorn app.main:app --host 0.0.0.0 --port 8000 --reload"
    Write-Host "`nPara o smoke test de WebSocket (precisa: pip install websockets):" -ForegroundColor Green
    Write-Host "  .venv\Scripts\python.exe scripts\ws_smoke.py"
}
finally {
    Pop-Location
}
