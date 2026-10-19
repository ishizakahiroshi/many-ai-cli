package provider

import (
	"fmt"
	"strings"
)

type LaunchRequest struct {
	Model     string
	Effort    string
	SessionID string
	Prompt    string
	Headless  bool
}

type ResolvedLaunch struct {
	ExecutableCandidates []string
	Args                 []string
	AllowedEnv           []string
	Revision             string
}

func EffortArgs(definition EffectiveDefinition, effort string) ([]string, error) {
	if definition.Launch == nil || effort == "" {
		return nil, nil
	}
	if len(definition.Launch.EffortLevels) > 0 {
		known := false
		for _, level := range definition.Launch.EffortLevels {
			if level == effort {
				known = true
				break
			}
		}
		if !known {
			return nil, fmt.Errorf("invalid effort %q for provider %q", effort, definition.ID)
		}
	}
	return expandArgs(definition.Launch.EffortArgs, map[string]string{"effort": effort})
}

func ModelArgs(definition EffectiveDefinition, model string) ([]string, error) {
	if definition.Launch == nil || model == "" {
		return nil, nil
	}
	return expandArgs(definition.Launch.ModelArgs, map[string]string{"model": model})
}

// ResolveLaunch expands only the small placeholder set accepted by schema v1.
// It never invokes a shell and returns argv entries as separate strings.
func ResolveLaunch(definition EffectiveDefinition, request LaunchRequest) (ResolvedLaunch, error) {
	if definition.Launch == nil {
		return ResolvedLaunch{}, fmt.Errorf("provider %q has no launch definition", definition.ID)
	}
	launch := definition.Launch
	candidates := append([]string(nil), launch.ExecutableCandidates...)
	if launch.Executable != "" {
		candidates = append([]string{launch.Executable}, candidates...)
	}
	if len(candidates) == 0 {
		return ResolvedLaunch{}, fmt.Errorf("provider %q has no executable", definition.ID)
	}
	args := append([]string(nil), launch.Args...)
	if request.Headless && launch.Headless != nil {
		args = append(args, launch.Headless.Args...)
		if request.Prompt != "" && launch.Headless.PromptVia == "arg" {
			args = append(args, request.Prompt)
		}
	}
	if request.Model != "" {
		modelArgs, err := expandArgs(launch.ModelArgs, map[string]string{"model": request.Model})
		if err != nil {
			return ResolvedLaunch{}, err
		}
		args = append(args, modelArgs...)
	}
	if request.Effort != "" {
		effortArgs, err := expandArgs(launch.EffortArgs, map[string]string{"effort": request.Effort})
		if err != nil {
			return ResolvedLaunch{}, err
		}
		args = append(args, effortArgs...)
	}
	return ResolvedLaunch{
		ExecutableCandidates: candidates,
		Args:                 args,
		AllowedEnv:           append([]string(nil), launch.AllowedEnv...),
		Revision:             definition.Revision,
	}, nil
}

func expandArgs(args []string, values map[string]string) ([]string, error) {
	out := make([]string, 0, len(args))
	for _, arg := range args {
		value := arg
		for key, replacement := range values {
			value = strings.ReplaceAll(value, "{"+key+"}", replacement)
		}
		if strings.ContainsAny(value, "{}") {
			return nil, fmt.Errorf("argv contains an unresolved placeholder")
		}
		out = append(out, value)
	}
	return out, nil
}
