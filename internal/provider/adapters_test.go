package provider

import "testing"

func TestAdapterDescriptorsAreFixedAndSorted(t *testing.T) {
	descriptors := AdapterDescriptors()
	if len(descriptors) == 0 {
		t.Fatal("AdapterDescriptors returned no registered adapters")
	}
	for i, descriptor := range descriptors {
		if descriptor.Key == "" || descriptor.Kind == "" || descriptor.Version == "" {
			t.Fatalf("invalid descriptor %d: %#v", i, descriptor)
		}
		if i > 0 && descriptors[i-1].Key >= descriptor.Key {
			t.Fatalf("adapter descriptors are not sorted: %#v", descriptors)
		}
	}
	if DefaultAdapterCatalog().Has("approval:unknown-v1") {
		t.Fatal("unknown adapter unexpectedly accepted")
	}
}
