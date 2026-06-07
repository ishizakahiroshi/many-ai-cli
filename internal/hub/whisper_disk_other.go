//go:build !windows

package hub

func ensureDownloadRoom(_ string, _ int64) error {
	return nil
}
