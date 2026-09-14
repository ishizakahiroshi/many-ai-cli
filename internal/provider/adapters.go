package provider

import "sort"

type AdapterKind string

const (
	AdapterLaunch       AdapterKind = "launch"
	AdapterApproval     AdapterKind = "approval"
	AdapterHistory      AdapterKind = "history"
	AdapterUsage        AdapterKind = "usage"
	AdapterSubscription AdapterKind = "subscription"
	AdapterPermissions  AdapterKind = "permissions"
)

type AdapterDescriptor struct {
	Key      string      `json:"key"`
	Kind     AdapterKind `json:"kind"`
	Version  string      `json:"version"`
	Provider string      `json:"provider,omitempty"`
}

// AdapterDescriptors is the reviewable, fixed catalog exposed to diagnostics.
// Definitions may reference these keys, but they cannot name Go symbols, files,
// URLs, shared libraries, or scripts.
func AdapterDescriptors() []AdapterDescriptor {
	keys := DefaultAdapterCatalog().Keys
	result := make([]AdapterDescriptor, 0, len(keys))
	for key := range keys {
		parts := splitAdapterKey(key)
		result = append(result, AdapterDescriptor{Key: key, Kind: AdapterKind(parts[0]), Version: parts[1], Provider: parts[2]})
	}
	sort.Slice(result, func(i, j int) bool { return result[i].Key < result[j].Key })
	return result
}

func splitAdapterKey(key string) [3]string {
	parts := [3]string{}
	segments := splitN(key, ':', 3)
	if len(segments) > 0 {
		parts[0] = segments[0]
	}
	if len(segments) > 1 {
		version := segments[1]
		if dash := indexByte(version, '-'); dash >= 0 {
			parts[1] = version[dash+1:]
			parts[2] = version[:dash]
		} else {
			parts[1] = version
		}
	}
	return parts
}

func splitN(value string, separator byte, limit int) []string {
	result := make([]string, 0, limit)
	start := 0
	for i := 0; i < len(value) && len(result) < limit-1; i++ {
		if value[i] != separator {
			continue
		}
		result = append(result, value[start:i])
		start = i + 1
	}
	return append(result, value[start:])
}

func indexByte(value string, target byte) int {
	for i := range value {
		if value[i] == target {
			return i
		}
	}
	return -1
}
