//go:build !windows

package wrapper

func repairMojibakeUTF8(data []byte) []byte {
	return data
}
