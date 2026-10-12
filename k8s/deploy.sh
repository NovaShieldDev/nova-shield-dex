#!/usr/bin/env bash
# Nova DEX — Kubernetes Dağıtım Script'i
# Kullanım: ./k8s/deploy.sh [--dry-run]
set -euo pipefail

DRY_RUN="${1:-}"
KUBECTL="kubectl"
if [[ "$DRY_RUN" == "--dry-run" ]]; then
    KUBECTL="kubectl --dry-run=client"
    echo "[dry-run] Gerçek uygulama yapılmayacak."
fi

echo "==> Namespace oluşturuluyor..."
$KUBECTL apply -f k8s/namespace.yaml

echo "==> Secret kontrol ediliyor..."
if [[ ! -f k8s/secret.yaml ]]; then
    echo "HATA: k8s/secret.yaml bulunamadı."
    echo "      cp k8s/secret.yaml.example k8s/secret.yaml"
    echo "      Ardından değerleri düzenleyin ve tekrar çalıştırın."
    exit 1
fi
$KUBECTL apply -f k8s/secret.yaml

echo "==> PostgreSQL dağıtılıyor..."
$KUBECTL apply -f k8s/postgres.yaml

echo "==> Redis dağıtılıyor..."
$KUBECTL apply -f k8s/redis.yaml

echo "==> PostgreSQL hazır olana kadar bekleniyor..."
if [[ "$DRY_RUN" != "--dry-run" ]]; then
    kubectl wait --for=condition=ready pod -l app=postgres -n nova-dex --timeout=120s
fi

echo "==> Nova DEX API sunucusu dağıtılıyor..."
$KUBECTL apply -f k8s/nova-dex-server.yaml

echo "==> Nova DEX Web arayüzü dağıtılıyor..."
$KUBECTL apply -f k8s/nova-dex-web.yaml

echo "==> Ingress dağıtılıyor..."
$KUBECTL apply -f k8s/ingress.yaml

echo ""
echo "✓ Dağıtım tamamlandı."
echo ""
echo "Pod durumlarını kontrol edin:"
echo "  kubectl get pods -n nova-dex"
echo ""
echo "HPA durumunu izleyin:"
echo "  kubectl get hpa -n nova-dex -w"
